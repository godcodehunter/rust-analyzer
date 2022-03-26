use std::sync::Arc;

use base_db::{FileId, SourceDatabaseExt, SourceRoot, Upcast, salsa};
use either::Either;
use hir::{self, Crate, Function, HasAttrs, HasSource, ModuleDef, Semantics, db::HirDatabase};
use rustc_hash::FxHashMap;
use syntax::{AstNode, TextRange, ast::{self, HasAttrs as _}};
use std::collections::LinkedList;

/// Defines the kind of [RunnableFunc]
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum RunnableFuncKind {
    /// The [unit test function](https://doc.rust-lang.org/reference/attributes/testing.html?highlight=test#testing-attributes),
    /// i.e. function marked with `#[test]` attribute and whose signature satisfies requirements.
    Test,
    /// The [benchmark test function](https://doc.rust-lang.org/unstable-book/library-features/test.html),
    /// i.e. function marked with `#[bench]` attribute and whose signature satisfies requirements.
    /// Requires the unstable feature `test` to be enabled.
    Bench,
    /// It is the entry point of the crate. Default is a function with the name `main` 
    /// that signature satisfies requirements. If unstable feature 
    /// [`start`](https://doc.rust-lang.org/unstable-book/language-features/start.html?highlight=start#start)
    /// enabled, insted use function market with attribute `#[start]` that signature satisfies requirements. 
    Bin,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct DoctestLocation {
    file_id: FileId,
    range: TextRange,
}

/// [Documentation tests](https://doc.rust-lang.org/rustdoc/documentation-tests.html)
/// these are special inserts into mardown that contain Rust code and can be executed 
/// as tests.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Doctest {
    pub location: DoctestLocation,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct RunnableFunc {
    pub kind: RunnableFuncKind,
    pub location: Function,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Runnable {
    Function(RunnableFunc),
    Doctest(Doctest),
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct MacroCall {
    call: (),
    content: LinkedList<RunnableView>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Module {
    pub location: hir::Module,
    content: LinkedList<RunnableView>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Node {
    MacroCall(MacroCall),
    Module(Module),
}

pub type Id = usize;

// trait ChangeObserver<T, U> {
//     fn delete(&mut self, target: &T, item: &T);
//     fn append(&mut self, target: &T, item: &T);
//     fn update(&mut self, target: &T, update: U);
// }

// struct DeltaSynchronizer {
//     patch: rust_analyzer::lsp_ext::Patch,
// }

// impl ChangeObserver<RunnableView, > for DeltaSynchronizer {
//     fn delete(&mut self, target: &RunnableView, item: &RunnableView) {
//         self.patch.delete.push(item as usize);
//     }

//     fn append(&mut self, target: &RunnableView, item: &RunnableView) {
//         todo!()
//     }

//     fn update(&mut self,) {
//         todo!()
//     }
// }


/// We can think about that tree as of a representation a partial view from AST. 
/// The main purpose why we need a partial view is that reduce the 
/// time to traverse a full tree. 
/// That is, this is part of the original tree containing the runnables and branches to them.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum RunnableView {
    Node(Node),
    Leaf(Runnable),
}

pub enum DefKey<'a> {
    Module(&'a hir::Module),
    Function(&'a Function),
}

impl<'a> From<&'a hir::Module> for DefKey<'a> {
    fn from(i: &'a hir::Module) -> Self {
        DefKey::Module(i)
    }
}

impl<'a> From<&'a Function> for DefKey<'a> {
    fn from(i: &'a Function) -> Self {
        DefKey::Function(i)
    }
}

impl RunnableView {
    pub fn get_by_def<'a, Key>(&self, key: Key) -> Option<&RunnableView> 
        where Key: Into<DefKey<'a>> {
        let def = &key.into();

        let mut ret = None;
        Self::dfs(self, |it| {
            match (def, it) {
                (DefKey::Function(key), RunnableView::Leaf(Runnable::Function(func))) => {
                    if func.location == **key {
                        ret = Some(it);
                        return true
                    }
                    false
                }
                (DefKey::Module(key), RunnableView::Node(Node::Module(node))) => {
                    if node.location == **key {
                        ret = Some(it);
                        return true
                    }
                    false
                }
                _ => false,
            }
        });
        ret
    }

    // Just DFS algorithm, that accepts tree root and handler function.
    // Handler function return false for continue crawling or true for stop it. 
    fn dfs<'a>(root: &'a RunnableView, mut handler: impl FnMut(&'a RunnableView) -> bool) {
        let mut buff = vec![root];
        while let Some(item) = buff.pop() {
            match item {
                RunnableView::Node(Node::Module(m)) => buff.extend(m.content.iter()),
                RunnableView::Node(Node::MacroCall(mc)) => buff.extend(mc.content.iter()),
                _ =>{},
            }   
            if handler(item) {break}
        }
    }

    // Returns an iterator over the contents of a file. 
    // Note: not including the root of the file.
    pub fn flatten_content(&self) -> impl Iterator<Item = &RunnableView> {
        let mut res = Vec::new();
        Self::dfs(self, |i| {
            if i != self {
                res.push(i);
            }
            false
        });
        res.into_iter()
    }

    pub fn get_by_id(&self, id: Id) -> Option<&RunnableView> {
        todo!()
    }
}

pub type WorkspaceRunnables = FxHashMap<Crate, Arc<CrateRunnables>>;
type CrateRunnables = FxHashMap<FileId, Arc<RunnableView>>;

// TODO: Dirty code, probably it should be, for example, member of [hir::Crate] 
fn crate_source_root(db: &dyn RunnableDatabase, krate: Crate) -> Arc<SourceRoot> {
    let module = krate.root_module(db.upcast());
    let file_id = module.definition_source(db.upcast()).file_id;
    let file_id = file_id.original_file(db.upcast());
    let source_root_id = db.file_source_root(file_id);
    db.source_root(source_root_id)
}

#[salsa::query_group(RunnableDatabaseStorage)]
pub trait RunnableDatabase: hir::db::HirDatabase + Upcast<dyn hir::db::HirDatabase> + SourceDatabaseExt {
    fn workspace_runnables(&self) -> Arc<WorkspaceRunnables>;
    fn crate_runnables(&self, krait: Crate) -> Arc<CrateRunnables>;
    fn file_runnables(&self, file_id: FileId) -> Option<Arc<RunnableView>>;
}

fn workspace_runnables(db: &dyn RunnableDatabase) -> Arc<WorkspaceRunnables> {
    let _p = profile::span("workspace_runnables");

    let mut res = WorkspaceRunnables::default();
    for krate in Crate::all(db.upcast()) {
        // Excludes libraries and process only what is relevant to the working project
        if !crate_source_root(db, krate).is_library {
            res.insert(krate, db.crate_runnables(krate)); 
        }
    }
    Arc::new(res)
}

fn crate_runnables(db: &dyn RunnableDatabase, krate: Crate) -> Arc<CrateRunnables> {
    let _p = profile::span("crate_runnables");

    let source_root = crate_source_root(db, krate);
    
    let mut res = CrateRunnables::default();
    for file_id in source_root.iter() {
        if let Some(runnables) = db.file_runnables(file_id) {
            res.insert(file_id, runnables);
        }
    }
    Arc::new(res)
}

fn file_runnables(db: &dyn RunnableDatabase, file_id: FileId) -> Option<Arc<RunnableView>> {
    struct Bijection {
        origin: hir::Module, 
        accord: Option<*mut Module>,
    }

    type MutalPath = Vec<Bijection>;

    // Represents the point from which paths begin to differ
    struct DifferencePoint(usize);

    // Compares paths and returns [DifferencePoint] if they are not equvalent 
    fn find_diff_point(path: &MutalPath) -> Option<DifferencePoint> {
        for item in path.into_iter().enumerate() {
            if item.1.accord.is_none() {
                return Some(DifferencePoint(item.0));
            }
        }

        None
    }

    // Reconstructs [RunnableView] branch and maintains consistency [MutalPath] 
    // in the process.
    fn syn_branches<'path>(
        path: &'path mut MutalPath, 
        dvg_point: &DifferencePoint
    ) {
        let mut iter = path.iter_mut().skip(dvg_point.0 - 1);
        let last_sync = iter.next().unwrap();

        iter.fold(last_sync, |cur: &mut Bijection, next: &mut Bijection| -> &mut Bijection {
            let node = Node::Module(Module{ 
                location: next.origin, 
                content: Default::default(),
            });
            unsafe {
                let content = &mut (*cur.accord.unwrap()).content;
                content.push_back(RunnableView::Node(node));
                if let RunnableView::Node(Node::Module(ref mut m))= content.back_mut().unwrap() {
                    next.accord = Some(m);
                }
            }
            next
        });
    }

    fn visit_file_defs_with_path(
        db: &dyn RunnableDatabase,
        sema: &Semantics,
        file_id: FileId,
        mut callback: impl FnMut(&dyn RunnableDatabase, &Semantics, &mut MutalPath, Either<hir::ModuleDef, hir::Impl>),
    ) {
        let module = match sema.to_module_def(file_id) {
            Some(it) => it,
            None => return,
        };
        
        let mut path = MutalPath::new();

        let declarations = module.declarations(sema.db);
        if declarations.is_empty() {
            return;
        }
        path.push(Bijection { origin: module, accord: None});
        let mut walk_queue: Vec<(hir::Module, Vec<ModuleDef>)> = vec![(module, declarations)];

        while let Some((parent, childrens)) = walk_queue.last_mut(){
            let parent = parent.clone();
            let defenition = childrens.pop().unwrap();
            if childrens.is_empty() {
                walk_queue.pop().unwrap().0;
            }
            
            // The end of path must be parent if the path end is different node 
            // then we crawl another branch. So, for getting the actual path we should 
            // drop old parts. 
            while path.last().unwrap().origin != parent {
                path.pop();
            }
          
            callback(db, sema, &mut path, Either::Left(defenition));  
            if let ModuleDef::Module(module) = defenition {
                for impl_ in module.impl_defs(sema.db) {
                    callback(db, sema, &mut path, Either::Right(impl_))
                }
            }

            if let ModuleDef::Module(module) = defenition {
                if let hir::ModuleSource::Module(_) = module.definition_source(sema.db).value {
                    let declartions = module.declarations(sema.db);
                    if !declartions.is_empty() {
                        path.push(Bijection { origin: module, accord: None});
                        walk_queue.push((module, declartions));
                    }
                }
            }
        }
    }

    fn store_runnables(res: &mut Option<RunnableView>, path: &mut MutalPath, runnables: &[Runnable]) {
        let mut diff_point = find_diff_point(path);
        
        // If result runnable view is empty, then initialize it's root node
        if let Some(ref point) = diff_point {
            if point.0 == 0 {
                let mut first = path.first_mut().unwrap();
                
                res.replace(RunnableView::Node(Node::Module(Module{
                    location: first.origin,
                    content: Default::default(),
                })));

                match res.as_mut().unwrap() {
                    RunnableView::Node(Node::Module(m)) => first.accord = Some(m),
                    _ => unreachable!(),
                }

                if path.len() == 1 {
                    diff_point = None;
                } else {
                    diff_point = Some(DifferencePoint(1));
                }
            }
        }

        if let Some(ref dvg_point) = diff_point {
            syn_branches(path, dvg_point);
        }
        
        unsafe {
            let content = &mut (*path.last_mut().unwrap().accord.unwrap()).content;
            
            content.extend(runnables.into_iter().map(|i| RunnableView::Leaf(i.clone())));
        }
    }

    fn is_from_macro(db: &dyn HirDatabase, def: &ModuleDef) -> bool {
        let file_id = match def {
            hir::ModuleDef::Module(it) => it.declaration_source(db).map(|src| src.file_id),
            hir::ModuleDef::Function(it) => it.source(db).map(|src| src.file_id),
            _ => return false,
        }; 
        file_id.map(|file| file.call_node(db.upcast())).is_some()
    }

    let _p = profile::span("file_runnables");

    let sema = Semantics::new(db.upcast());

    let mut res = None;
    
    visit_file_defs_with_path(db, &sema, file_id,
        |
            db: &dyn RunnableDatabase, 
            sema: &Semantics, 
            path: &mut MutalPath, 
            def: Either<hir::ModuleDef, hir::Impl>
        | {
        // TODO: vector of static size 2 on the stack 
        let mut runnables = vec![];
        if let Some(doctest) = match def {
            Either::Left(m) => match m {
                ModuleDef::Module(i) => has_doctest(db.upcast(), i),
                ModuleDef::Function(i) => has_doctest(db.upcast(), i),
                ModuleDef::Adt(i) => has_doctest(db.upcast(), i),
                ModuleDef::Variant(i) => has_doctest(db.upcast(), i),
                ModuleDef::Const(i) => has_doctest(db.upcast(), i),
                ModuleDef::Static(i) => has_doctest(db.upcast(), i),
                ModuleDef::Trait(i) => has_doctest(db.upcast(), i),
                ModuleDef::TypeAlias(i) => has_doctest(db.upcast(), i),
                ModuleDef::BuiltinType(_) => None,
            },
            Either::Right(_impl) => has_doctest(db.upcast(), _impl),
        } {
            runnables.push(doctest);
        }

        if let Some(function) = match def {
            Either::Left(hir::ModuleDef::Function(it)) => runnable_fn(&sema, it),
            _ => None,
        } {
            runnables.push(function);
        }
        
        if !runnables.is_empty() {
            store_runnables(&mut res, path, &runnables);
        }
    });

    // sema.to_module_defs(file_id)
    //     .map(|it| runnable_mod_outline_definition(&sema, it))
    //     .for_each(|it| add_opt(it, None));

    // res.extend(in_macro_expansion.into_iter().flat_map(|(_, runnables)| {
    //     let use_name_in_title = runnables.len() != 1;
    //     runnables.into_iter().map(move |mut r| {
    //         r.use_name_in_title = use_name_in_title;
    //         r
    //     })
    // }));

    res.map(Arc::new)
}

fn validate_main_signature() {
    // enum ValidationResult {
    //     // Occurrence when function signature is incomplete
    //     Unknown,
    //     // Non-compliance error detected
    //     Error,
    //     // Function signature satisfy requirements
    //     Valid,
    // }

    // //  Checking if functions signature equal one of following:
    // //  if found trait std::process::Termination:
    // //      'fn() -> impl Termination'
    // //  in other case:
    // //      'fn() -> ()'
    // //      'fn() -> Result<(), E> where E: Error'
    // // 
    // // TODO: check multiple definitions 
    // // TRACK: when [RFC 1937](https://github.com/rust-lang/rust/issues/43301) stabilized,
    // // and the trait will be moved to lib core, the function should rely entirely on trait 
    // // searching and check return type for conformation to it 
    // let validate_signature = |fn_def: &ast::FnDef| -> ValidationResult {
    //     let type_param = fn_def.type_param_list();
    //     if type_param.is_some() {
    //         return ValidationResult::Error;
    //     }
    //     if fn_def.where_clause().is_some() {
    //         return ValidationResult::Error;
    //     }

    //     let par_list = fn_def.param_list(); 
    //     if par_list.is_none() {
    //         return ValidationResult::Unknown;
    //     }        
    //     let par_list = par_list.unwrap();       
    //     let par_num = par_list.params().count();
    //     let is_have_self = par_list.self_param().is_some();        
    //     if par_num != 0 || is_have_self {
    //         return ValidationResult::Error;
    //     }
        
    //     if fn_def.ret_type().is_none() {
    //         return ValidationResult::Unknown;
    //     }
    //     let ret_type = fn_def.ret_type().unwrap();
    //     let type_ref = ret_type.type_ref();
    //     if type_ref.is_none() {
    //         return ValidationResult::Valid; 
    //     }
    //     let type_ref = type_ref.unwrap();
        
    //     let module = sema.to_def(fn_def).unwrap().module(sema.db).to_source();
    //     let attrs = Attrs::from_attrs_owner(sema.db, module);
    //     let features = attrs.by_key("feature");

    //     // TODO: Candidate search the whole project, separate it 
        
    //     ValidationResult::Valid
    // };
}

fn validate_start_signature() {
    todo!()
}

fn validate_bench_signature() {
    todo!()
}

/// Creates a test mod runnable for outline modules at the top of their definition.
fn runnable_mod_outline_definition(
    sema: &Semantics,
    def: hir::Module,
) -> Option<RunnableView> {
    // if !is_contains_runnable(sema, &def) {
    //     return None;
    // }
    // let path = def.path_to_root(sema.db).into_iter().rev().filter_map(|it| it.name(sema.db)).join("::");

    // let attrs = def.attrs(sema.db);
    // let cfg = attrs.cfg();
    // match def.definition_source(sema.db).value {
    //     hir::ModuleSource::SourceFile(_) => Some(Runnable {
    //         use_name_in_title: false,
    //         nav: def.to_nav(sema.db),
    //         kind: RunnableKind::TestMod { path },
    //         cfg,
    //     }),
    //     _ => None,
    // }

    // Some(RunnableView::Module{ location: def, content: ()})
    todo!()
}

/// Checks if item containe runnable in doc than create [Runnable] from it
fn has_doctest<AtrOwner: HasAttrs>(db: &dyn HirDatabase, attrs_onwer: AtrOwner) -> Option<Runnable> {
    if !is_contains_runnable_in_doc(&*attrs_onwer.attrs(db)) {
        return None;
    }

    Some(Runnable::Doctest(Doctest{ location: todo!() }))
}

/// Checks if a [hir::Function] is runnable and if it is, then construct [Runnable] from it 
fn runnable_fn(sema: &Semantics, def: hir::Function) -> Option<Runnable> {
    let func = def.source(sema.db)?;
    let name_string = def.name(sema.db).to_string();

    let root = def.module(sema.db).krate().root_module(sema.db);

    let kind = if name_string == "main" && def.module(sema.db) == root {
        RunnableFuncKind::Bin
    } else {
        if extract_test_related_attribute(&func.value).is_some() {
            RunnableFuncKind::Test
        } else if func.value.has_atom_attr("bench") {
            RunnableFuncKind::Bench
        } else {
            return None;
        }
    };

    Some(Runnable::Function(RunnableFunc{ kind, location: def }))
}

/// This is a method with a heuristics to support test methods annotated 
/// with custom test annotations, such as `#[test_case(...)]`, 
/// `#[tokio::test]` and similar.
/// Also a regular `#[test]` annotation is supported.
///
/// It may produce false positives, for example, `#[wasm_bindgen_test]` 
/// requires a different command to run the test, but it's better than 
/// not to have the runnables for the tests at all.
pub fn extract_test_related_attribute(fn_def: &ast::Fn) -> Option<ast::Attr> {
    fn_def.attrs().find_map(|attr| {
        attr.path()?
            .syntax()
            .text()
            .to_string()
            .contains("test")
            .then(|| attr)
    })
}

const RUSTDOC_FENCE: &str = "```";
const RUSTDOC_CODE_BLOCK_ATTRIBUTES_RUNNABLE: &[&str] =
    &["", "rust", "should_panic", "edition2015", "edition2018", "edition2021"];

/// Checks that the attributes contain documentation that contain 
/// specially formed code blocks 
fn is_contains_runnable_in_doc(attrs: &hir::Attrs) -> bool {
    attrs.docs().map_or(false, |doc| {
        for line in String::from(doc).lines() {
            if let Some(header) = line.strip_prefix(RUSTDOC_FENCE) {
                if header
                        .split(',')
                        .all(|sub| RUSTDOC_CODE_BLOCK_ATTRIBUTES_RUNNABLE.contains(&sub.trim()))
                {
                    return true;
                }
            }
        }

        false
    })
}