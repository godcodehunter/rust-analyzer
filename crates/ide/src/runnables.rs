use std::fmt;

use ast::HasName;
use cfg::CfgExpr;
use either::Either;
use hir::{AsAssocItem, HasAttrs, HasSource, HirDisplay, Semantics};
use ide_db::{RootDatabase, SymbolKind, base_db::{FilePosition, FileRange, Upcast}, helpers::visit_file_defs, search::SearchScope};
use itertools::Itertools;
use rustc_hash::{FxHashMap, FxHashSet};
use stdx::{always, format_to};
use syntax::ast::{self, AstNode, HasAttrs as _};

use crate::{
    display::{ToNav, TryToNav},
    references, FileId, NavigationTarget,
};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Runnable {
    pub use_name_in_title: bool,
    pub nav: NavigationTarget,
    pub kind: RunnableKind,
    pub cfg: Option<CfgExpr>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum TestId {
    Name(String),
    Path(String),
}

impl fmt::Display for TestId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TestId::Name(name) => write!(f, "{}", name),
            TestId::Path(path) => write!(f, "{}", path),
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum RunnableKind {
    Test { test_id: TestId, attr: TestAttr },
    TestMod { path: String },
    Bench { test_id: TestId },
    DocTest { test_id: TestId },
    Bin,
}

#[cfg(test)]
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum RunnableTestKind {
    Test,
    TestMod,
    DocTest,
    Bench,
    Bin,
}

impl Runnable {
    // test package::module::testname
    pub fn label(&self, target: Option<String>) -> String {
        match &self.kind {
            RunnableKind::Test { test_id, .. } => format!("test {}", test_id),
            RunnableKind::TestMod { path } => format!("test-mod {}", path),
            RunnableKind::Bench { test_id } => format!("bench {}", test_id),
            RunnableKind::DocTest { test_id, .. } => format!("doctest {}", test_id),
            RunnableKind::Bin => {
                target.map_or_else(|| "run binary".to_string(), |t| format!("run {}", t))
            }
        }
    }

    pub fn title(&self) -> String {
        let mut s = String::from("▶\u{fe0e} Run ");
        if self.use_name_in_title {
            format_to!(s, "{}", self.nav.name);
            if !matches!(self.kind, RunnableKind::Bin) {
                s.push(' ');
            }
        }
        let suffix = match &self.kind {
            RunnableKind::TestMod { .. } => "Tests",
            RunnableKind::Test { .. } => "Test",
            RunnableKind::DocTest { .. } => "Doctest",
            RunnableKind::Bench { .. } => "Bench",
            RunnableKind::Bin => return s,
        };
        s.push_str(suffix);
        s
    }

    #[cfg(test)]
    fn test_kind(&self) -> RunnableTestKind {
        match &self.kind {
            RunnableKind::TestMod { .. } => RunnableTestKind::TestMod,
            RunnableKind::Test { .. } => RunnableTestKind::Test,
            RunnableKind::DocTest { .. } => RunnableTestKind::DocTest,
            RunnableKind::Bench { .. } => RunnableTestKind::Bench,
            RunnableKind::Bin => RunnableTestKind::Bin,
        }
    }
}

impl Runnable {
    pub fn from_db_repr(sema: &Semantics, runnable: ide_db::runnables::RunnableView) -> Self {
        match runnable {
            ide_db::runnables::RunnableView::Module { location, content} 
                => from_mod(sema, runnable),
            ide_db::runnables::RunnableView::Function(runnable)
                => from_fn(sema, runnable),
            ide_db::runnables::RunnableView::Doctest(_) 
                => todo!(),
        }
    }
    
    fn from_mod(sema: &Semantics, def: hir::Module) -> Runnable {
        //TODO: replace by [hir::ModuleDef::canonical_path]
        let path = 
            def.path_to_root(sema.db).into_iter().rev().filter_map(|it| it.name(sema.db)).join("::");
        let cfg = def.attrs(sema.db).cfg();
        let nav = NavigationTarget::from_module_to_decl(sema.db, def);
        Runnable { 
            use_name_in_title: false, 
            nav, 
            kind: RunnableKind::TestMod { path }, 
            cfg 
        }
    }
    
    fn from_fn(sema: &Semantics, def: hir::Function) -> Runnable {
        let cfg = def.attrs(sema.db).cfg();
        let nav = NavigationTarget::from_named(
            sema.db,
            func.as_ref().map(|it| it as &dyn ast::NameOwner),
            SymbolKind::Function,
        );
        Runnable { use_name_in_title: false, nav, kind, cfg }
    }
}

// Feature: Run
//
// Shows a popup suggesting to run a test/benchmark/binary **at the current cursor
// location**. Super useful for repeatedly running just a single test. Do bind this
// to a shortcut!
//
// |===
// | Editor  | Action Name
//
// | VS Code | **Rust Analyzer: Run**
// |===
// image::https://user-images.githubusercontent.com/48062697/113065583-055aae80-91b1-11eb-958f-d67efcaf6a2f.gif[]
pub(crate) fn runnables(db: &dyn RunnableDatabase, file_id: FileId) -> Vec<Runnable> {
    db.file_runnables(file_id)
}

// Feature: Related Tests
//
// Provides a sneak peek of all tests where the current item is used.
//
// The simplest way to use this feature is via the context menu:
//  - Right-click on the selected item. The context menu opens.
//  - Select **Peek related tests**
//
// |===
// | Editor  | Action Name
//
// | VS Code | **Rust Analyzer: Peek related tests**
// |===
pub(crate) fn related_tests(
    db: &RootDatabase,
    position: FilePosition,
    search_scope: Option<SearchScope>,
) -> Vec<Runnable> {
    let sema = Semantics::new(db);
    let mut res: FxHashSet<Runnable> = FxHashSet::default();

    find_related_tests(&sema, position, search_scope, &mut res);

    res.into_iter().collect_vec()
}

fn find_related_tests(
    sema: &Semantics,
    position: FilePosition,
    search_scope: Option<SearchScope>,
    tests: &mut FxHashSet<Runnable>,
) {
    if let Some(refs) = references::find_all_refs(sema, position, search_scope) {
        for (file_id, refs) in refs.into_iter().flat_map(|refs| refs.references) {
            let file = sema.parse(file_id);
            let file = file.syntax();

            // create flattened vec of tokens
            let tokens = refs.iter().flat_map(|(range, _)| {
                match file.token_at_offset(range.start()).next() {
                    Some(token) => sema.descend_into_macros(token),
                    None => Default::default(),
                }
            });

            // find first suitable ancestor
            let functions = tokens
                .filter_map(|token| token.ancestors().find_map(ast::Fn::cast))
                .map(|f| hir::InFile::new(sema.hir_file_for(f.syntax()), f));

            for fn_def in functions {
                // #[test/bench] expands to just the item causing us to lose the attribute, so recover them by going out of the attribute
                let InFile { value: fn_def, .. } = &fn_def.node_with_attributes(sema.db);
                if let Some(runnable) = as_test_runnable(sema, fn_def) {
                    // direct test
                    tests.insert(runnable);
                } else if let Some(module) = parent_test_module(sema, fn_def) {
                    // indirect test
                    find_related_tests_in_module(sema, fn_def, &module, tests);
                }
            }
        }
    }
}
fn find_related_tests_in_module(
    sema: &Semantics<RootDatabase>,
    fn_def: &ast::Fn,
    parent_module: &hir::Module,
    tests: &mut FxHashSet<Runnable>,
) {
    if let Some(fn_name) = fn_def.name() {
        let mod_source = parent_module.definition_source(sema.db);
        let range = match mod_source.value {
            hir::ModuleSource::Module(m) => m.syntax().text_range(),
            hir::ModuleSource::BlockExpr(b) => b.syntax().text_range(),
            hir::ModuleSource::SourceFile(f) => f.syntax().text_range(),
        };

        let file_id = mod_source.file_id.original_file(sema.db);
        let mod_scope = SearchScope::file_range(FileRange { file_id, range });
        let fn_pos = FilePosition { file_id, offset: fn_name.syntax().text_range().start() };
        find_related_tests(sema, fn_pos, Some(mod_scope), tests)
    }
}

fn as_test_runnable(sema: &Semantics<RootDatabase>, fn_def: &ast::Fn) -> Option<Runnable> {
    if test_related_attribute(fn_def).is_some() {
        let function = sema.to_def(fn_def)?;
        runnable_fn(sema, function)
    } else {
        None
    }
}

fn parent_test_module(sema: &Semantics<RootDatabase>, fn_def: &ast::Fn) -> Option<hir::Module> {
    fn_def.syntax().ancestors().find_map(|node| {
        let module = ast::Module::cast(node)?;
        let module = sema.to_def(&module)?;

        if is_contains_runnable(sema, &module) {
            Some(module)
        } else {
            None
        }
    })
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct TestAttr {
    pub ignore: bool,
}

impl TestAttr {
    fn from_fn(fn_def: &ast::Fn) -> TestAttr {
        let ignore = fn_def
            .attrs()
            .filter_map(|attr| attr.simple_name())
            .any(|attribute_text| attribute_text == "ignore");
        TestAttr { ignore }
    }
}

#[cfg(test)]
mod tests {
    use expect_test::{expect, Expect};

    use crate::fixture;

    use super::{RunnableTestKind::*, *};

    fn check(
        ra_fixture: &str,
        // FIXME: fold this into `expect` as well
        actions: &[RunnableTestKind],
        expect: Expect,
    ) {
        let (analysis, position) = fixture::position(ra_fixture);
        let runnables = analysis.runnables(position.file_id).unwrap();
        expect.assert_debug_eq(&runnables);
        assert_eq!(
            actions,
            runnables.into_iter().map(|it| it.test_kind()).collect::<Vec<_>>().as_slice()
        );
    }

    fn check_tests(ra_fixture: &str, expect: Expect) {
        let (analysis, position) = fixture::position(ra_fixture);
        let tests = analysis.related_tests(position, None).unwrap();
        expect.assert_debug_eq(&tests);
    }

    #[test]
    fn test_runnables() {
        check(
            r#"
//- /lib.rs
$0
fn main() {}

#[test]
fn test_foo() {}

#[test]
#[ignore]
fn test_foo() {}

#[bench]
fn bench() {}

mod not_a_root {
    fn main() {}
}
"#,
            &[Bin, Test, Test, Bench, TestMod],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..13,
                            focus_range: 4..8,
                            name: "main",
                            kind: Function,
                        },
                        kind: Bin,
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 15..39,
                            focus_range: 26..34,
                            name: "test_foo",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "test_foo",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 41..75,
                            focus_range: 62..70,
                            name: "test_foo",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "test_foo",
                            ),
                            attr: TestAttr {
                                ignore: true,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 77..99,
                            focus_range: 89..94,
                            name: "bench",
                            kind: Function,
                        },
                        kind: Bench {
                            test_id: Path(
                                "bench",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 0..137,
                            name: "",
                            kind: Module,
                        },
                        kind: TestMod {
                            path: "",
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_doc_test() {
        check(
            r#"
//- /lib.rs
$0
fn main() {}

/// ```
/// let x = 5;
/// ```
fn should_have_runnable() {}

/// ```edition2018
/// let x = 5;
/// ```
fn should_have_runnable_1() {}

/// ```
/// let z = 55;
/// ```
///
/// ```ignore
/// let z = 56;
/// ```
fn should_have_runnable_2() {}

/**
```rust
let z = 55;
```
*/
fn should_have_no_runnable_3() {}

/**
    ```rust
    let z = 55;
    ```
*/
fn should_have_no_runnable_4() {}

/// ```no_run
/// let z = 55;
/// ```
fn should_have_no_runnable() {}

/// ```ignore
/// let z = 55;
/// ```
fn should_have_no_runnable_2() {}

/// ```compile_fail
/// let z = 55;
/// ```
fn should_have_no_runnable_3() {}

/// ```text
/// arbitrary plain text
/// ```
fn should_have_no_runnable_4() {}

/// ```text
/// arbitrary plain text
/// ```
///
/// ```sh
/// $ shell code
/// ```
fn should_have_no_runnable_5() {}

/// ```rust,no_run
/// let z = 55;
/// ```
fn should_have_no_runnable_6() {}

/// ```
/// let x = 5;
/// ```
struct StructWithRunnable(String);

/// ```
/// let x = 5;
/// ```
impl StructWithRunnable {}

trait Test {
    fn test() -> usize {
        5usize
    }
}

/// ```
/// let x = 5;
/// ```
impl Test for StructWithRunnable {}
"#,
            &[Bin, DocTest, DocTest, DocTest, DocTest, DocTest, DocTest, DocTest, DocTest],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..13,
                            focus_range: 4..8,
                            name: "main",
                            kind: Function,
                        },
                        kind: Bin,
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 15..74,
                            name: "should_have_runnable",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "should_have_runnable",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 76..148,
                            name: "should_have_runnable_1",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "should_have_runnable_1",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 150..254,
                            name: "should_have_runnable_2",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "should_have_runnable_2",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 256..320,
                            name: "should_have_no_runnable_3",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "should_have_no_runnable_3",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 322..398,
                            name: "should_have_no_runnable_4",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "should_have_no_runnable_4",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 900..965,
                            name: "StructWithRunnable",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "StructWithRunnable",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 967..1024,
                            focus_range: 1003..1021,
                            name: "impl",
                            kind: Impl,
                        },
                        kind: DocTest {
                            test_id: Path(
                                "StructWithRunnable",
                            ),
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1088..1154,
                            focus_range: 1133..1151,
                            name: "impl",
                            kind: Impl,
                        },
                        kind: DocTest {
                            test_id: Path(
                                "StructWithRunnable",
                            ),
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_doc_test_in_impl() {
        check(
            r#"
//- /lib.rs
$0
fn main() {}

struct Data;
impl Data {
    /// ```
    /// let x = 5;
    /// ```
    fn foo() {}
}
"#,
            &[Bin, DocTest],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..13,
                            focus_range: 4..8,
                            name: "main",
                            kind: Function,
                        },
                        kind: Bin,
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 44..98,
                            name: "foo",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "Data::foo",
                            ),
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_module() {
        check(
            r#"
//- /lib.rs
$0
mod test_mod {
    #[test]
    fn test_foo1() {}
}
"#,
            &[TestMod, Test],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..51,
                            focus_range: 5..13,
                            name: "test_mod",
                            kind: Module,
                            description: "mod test_mod",
                        },
                        kind: TestMod {
                            path: "test_mod",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 20..49,
                            focus_range: 35..44,
                            name: "test_foo1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "test_mod::test_foo1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn only_modules_with_test_functions_or_more_than_one_test_submodule_have_runners() {
        check(
            r#"
//- /lib.rs
$0
mod root_tests {
    mod nested_tests_0 {
        mod nested_tests_1 {
            #[test]
            fn nested_test_11() {}

            #[test]
            fn nested_test_12() {}
        }

        mod nested_tests_2 {
            #[test]
            fn nested_test_2() {}
        }

        mod nested_tests_3 {}
    }

    mod nested_tests_4 {}
}
"#,
            &[TestMod, TestMod, TestMod, Test, Test, Test],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 22..323,
                            focus_range: 26..40,
                            name: "nested_tests_0",
                            kind: Module,
                            description: "mod nested_tests_0",
                        },
                        kind: TestMod {
                            path: "root_tests::nested_tests_0",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 51..192,
                            focus_range: 55..69,
                            name: "nested_tests_1",
                            kind: Module,
                            description: "mod nested_tests_1",
                        },
                        kind: TestMod {
                            path: "root_tests::nested_tests_0::nested_tests_1",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 202..286,
                            focus_range: 206..220,
                            name: "nested_tests_2",
                            kind: Module,
                            description: "mod nested_tests_2",
                        },
                        kind: TestMod {
                            path: "root_tests::nested_tests_0::nested_tests_2",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 84..126,
                            focus_range: 107..121,
                            name: "nested_test_11",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "root_tests::nested_tests_0::nested_tests_1::nested_test_11",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 140..182,
                            focus_range: 163..177,
                            name: "nested_test_12",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "root_tests::nested_tests_0::nested_tests_1::nested_test_12",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 235..276,
                            focus_range: 258..271,
                            name: "nested_test_2",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "root_tests::nested_tests_0::nested_tests_2::nested_test_2",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_with_feature() {
        check(
            r#"
//- /lib.rs crate:foo cfg:feature=foo
$0
#[test]
#[cfg(feature = "foo")]
fn test_foo1() {}
"#,
            &[Test, TestMod],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..50,
                            focus_range: 36..45,
                            name: "test_foo1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "test_foo1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: Some(
                            Atom(
                                KeyValue {
                                    key: "feature",
                                    value: "foo",
                                },
                            ),
                        ),
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 0..51,
                            name: "",
                            kind: Module,
                        },
                        kind: TestMod {
                            path: "",
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_with_features() {
        check(
            r#"
//- /lib.rs crate:foo cfg:feature=foo,feature=bar
$0
#[test]
#[cfg(all(feature = "foo", feature = "bar"))]
fn test_foo1() {}
"#,
            &[Test, TestMod],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..72,
                            focus_range: 58..67,
                            name: "test_foo1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "test_foo1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: Some(
                            All(
                                [
                                    Atom(
                                        KeyValue {
                                            key: "feature",
                                            value: "foo",
                                        },
                                    ),
                                    Atom(
                                        KeyValue {
                                            key: "feature",
                                            value: "bar",
                                        },
                                    ),
                                ],
                            ),
                        ),
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 0..73,
                            name: "",
                            kind: Module,
                        },
                        kind: TestMod {
                            path: "",
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_no_test_function_in_module() {
        check(
            r#"
//- /lib.rs
$0
mod test_mod {
    fn foo1() {}
}
"#,
            &[],
            expect![[r#"
                []
            "#]],
        );
    }

    #[test]
    fn test_doc_runnables_impl_mod() {
        check(
            r#"
//- /lib.rs
mod foo;
//- /foo.rs
struct Foo;$0
impl Foo {
    /// ```
    /// let x = 5;
    /// ```
    fn foo() {}
}
        "#,
            &[DocTest],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                1,
                            ),
                            full_range: 27..81,
                            name: "foo",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "foo::Foo::foo",
                            ),
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn test_runnables_in_macro() {
        check(
            r#"
//- /lib.rs
$0
macro_rules! gen {
    () => {
        #[test]
        fn foo_test() {}
    }
}
macro_rules! gen2 {
    () => {
        mod tests2 {
            #[test]
            fn foo_test2() {}
        }
    }
}
mod tests {
    gen!();
}
gen2!();
"#,
            &[TestMod, TestMod, TestMod, Test, Test],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 202..227,
                            focus_range: 206..211,
                            name: "tests",
                            kind: Module,
                            description: "mod tests",
                        },
                        kind: TestMod {
                            path: "tests",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 0..237,
                            name: "",
                            kind: Module,
                        },
                        kind: TestMod {
                            path: "",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 228..236,
                            focus_range: 228..236,
                            name: "tests2",
                            kind: Module,
                            description: "mod tests2",
                        },
                        kind: TestMod {
                            path: "tests2",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 228..236,
                            name: "foo_test2",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests2::foo_test2",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 218..225,
                            name: "foo_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn big_mac() {
        check(
            r#"
//- /lib.rs
$0
macro_rules! foo {
    () => {
        mod foo_tests {
            #[test]
            fn foo0() {}
            #[test]
            fn foo1() {}
            #[test]
            fn foo2() {}
        }
    };
}
foo!();
"#,
            &[TestMod, Test, Test, Test],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 210..217,
                            focus_range: 210..217,
                            name: "foo_tests",
                            kind: Module,
                            description: "mod foo_tests",
                        },
                        kind: TestMod {
                            path: "foo_tests",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 210..217,
                            name: "foo0",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "foo_tests::foo0",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 210..217,
                            name: "foo1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "foo_tests::foo1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 210..217,
                            name: "foo2",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "foo_tests::foo2",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn dont_recurse_in_outline_submodules() {
        check(
            r#"
//- /lib.rs
$0
mod m;
//- /m.rs
mod tests {
    #[test]
    fn t() {}
}
"#,
            &[],
            expect![[r#"
                []
            "#]],
        );
    }

    #[test]
    fn outline_submodule1() {
        check(
            r#"
//- /lib.rs
$0
mod m;
//- /m.rs
#[test]
fn t0() {}
#[test]
fn t1() {}
"#,
            &[TestMod],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 1..7,
                            focus_range: 5..6,
                            name: "m",
                            kind: Module,
                            description: "mod m",
                        },
                        kind: TestMod {
                            path: "m",
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn outline_submodule2() {
        check(
            r#"
//- /lib.rs
mod m;
//- /m.rs
$0
#[test]
fn t0() {}
#[test]
fn t1() {}
"#,
            &[Test, Test, TestMod],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                1,
                            ),
                            full_range: 1..19,
                            focus_range: 12..14,
                            name: "t0",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "m::t0",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                1,
                            ),
                            full_range: 20..38,
                            focus_range: 31..33,
                            name: "t1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "m::t1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                1,
                            ),
                            full_range: 0..39,
                            name: "m",
                            kind: Module,
                        },
                        kind: TestMod {
                            path: "m",
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn attributed_module() {
        check(
            r#"
//- proc_macros: identity
//- /lib.rs
$0
#[proc_macros::identity]
mod module {
    #[test]
    fn t0() {}
    #[test]
    fn t1() {}
}
"#,
            &[TestMod, Test, Test],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 26..94,
                            focus_range: 30..36,
                            name: "module",
                            kind: Module,
                            description: "mod module",
                        },
                        kind: TestMod {
                            path: "module",
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 43..65,
                            focus_range: 58..60,
                            name: "t0",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "module::t0",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: true,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 70..92,
                            focus_range: 85..87,
                            name: "t1",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "module::t1",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn find_no_tests() {
        check_tests(
            r#"
//- /lib.rs
fn foo$0() {  };
"#,
            expect![[r#"
                []
            "#]],
        );
    }

    #[test]
    fn find_direct_fn_test() {
        check_tests(
            r#"
//- /lib.rs
fn foo$0() { };

mod tests {
    #[test]
    fn foo_test() {
        super::foo()
    }
}
"#,
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 31..85,
                            focus_range: 46..54,
                            name: "foo_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn find_direct_struct_test() {
        check_tests(
            r#"
//- /lib.rs
struct Fo$0o;
fn foo(arg: &Foo) { };

mod tests {
    use super::*;

    #[test]
    fn foo_test() {
        foo(Foo);
    }
}
"#,
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 71..122,
                            focus_range: 86..94,
                            name: "foo_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn find_indirect_fn_test() {
        check_tests(
            r#"
//- /lib.rs
fn foo$0() { };

mod tests {
    use super::foo;

    fn check1() {
        check2()
    }

    fn check2() {
        foo()
    }

    #[test]
    fn foo_test() {
        check1()
    }
}
"#,
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 133..183,
                            focus_range: 148..156,
                            name: "foo_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn tests_are_unique() {
        check_tests(
            r#"
//- /lib.rs
fn foo$0() { };

mod tests {
    use super::foo;

    #[test]
    fn foo_test() {
        foo();
        foo();
    }

    #[test]
    fn foo2_test() {
        foo();
        foo();
    }

}
"#,
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 52..115,
                            focus_range: 67..75,
                            name: "foo_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 121..185,
                            focus_range: 136..145,
                            name: "foo2_test",
                            kind: Function,
                        },
                        kind: Test {
                            test_id: Path(
                                "tests::foo2_test",
                            ),
                            attr: TestAttr {
                                ignore: false,
                            },
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }

    #[test]
    fn doc_test_type_params() {
        check(
            r#"
//- /lib.rs
$0
struct Foo<T, U>;

impl<T, U> Foo<T, U> {
    /// ```rust
    /// ````
    fn t() {}
}
"#,
            &[DocTest],
            expect![[r#"
                [
                    Runnable {
                        use_name_in_title: false,
                        nav: NavigationTarget {
                            file_id: FileId(
                                0,
                            ),
                            full_range: 47..85,
                            name: "t",
                        },
                        kind: DocTest {
                            test_id: Path(
                                "Foo<T, U>::t",
                            ),
                        },
                        cfg: None,
                    },
                ]
            "#]],
        );
    }
}
