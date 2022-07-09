use std::sync::{Arc, Mutex};

use base_db::{salsa, FileId, SourceDatabaseExt, SourceRoot, Upcast};
use either::Either;
use hir::{self, db::HirDatabase, Crate, HasAttrs, HasSource, ModuleDef, Semantics};
use rustc_hash::FxHashMap;
use syntax::{
    ast::{self, HasAttrs as _},
    AstNode,
};
use lazy_static::lazy_static;

mod delta_patches;
mod runnable_view;
mod algo;

use runnable_view::*;
use delta_patches::*;
use algo::*;

pub use runnable_view::*;
pub use delta_patches::*;

// TODO: Dirty code, probably it should be, for example, member of [hir::Crate]
fn crate_source_root(db: &dyn RunnableDatabase, krate: Crate) -> Arc<SourceRoot> {
    let module = krate.root_module(db.upcast());
    let file_id = module.definition_source(db.upcast()).file_id;
    let file_id = file_id.original_file(db.upcast());
    let source_root_id = db.file_source_root(file_id);
    db.source_root(source_root_id)
}

lazy_static! {
    static ref PATCH: Mutex<Patch> =  Mutex::new(Patch::default());
}

#[salsa::query_group(RunnableDatabaseStorage)]
pub trait RunnableDatabase:
    hir::db::HirDatabase + Upcast<dyn hir::db::HirDatabase> + SourceDatabaseExt
{
    fn workspace_runnables(&self) -> Session;
    fn crate_runnables(&self, krait: Crate) -> Option<runnable_view::Crate>;
    fn file_runnables(&self, file_id: FileId) -> Option<runnable_view::Module>;
}

pub fn patch() -> &'static Mutex<Patch> {
    &*PATCH
}

fn workspace_runnables(db: &dyn RunnableDatabase) -> Session {
    let _p = profile::span("workspace_runnables");

    let mut session = Session{ crates: Default::default()};
    for krate in Crate::all(db.upcast()) {
        // Excludes libraries and process only what is relevant to the working project
        if !crate_source_root(db, krate).is_library {
            if let Some(krate) = db.crate_runnables(krate) {
                let mut patch = patch().lock().unwrap();
                let mut mutator = ItemMutator::new(RefNode::Session(&mut session), &mut patch);
                mutator.append(AppendItem::Crate(krate));
            }
        }
    }
    session
}

fn crate_runnables(db: &dyn RunnableDatabase, krate: Crate) -> Option<runnable_view::Crate> {
    let _p = profile::span("crate_runnables");

    let source_root = crate_source_root(db, krate);

    // TODO: why it is can be optional? 
    let name = if let Some(name) = krate.display_name(db.upcast()) {
        name.to_string()
    } else {
        "UNKNOW_CRATE_NAME".to_string()
    };

    let mut res = runnable_view::Crate{ 
        id: uuid::Uuid::new_v4().as_u128(), 
        name, 
        modules: Default::default(),
    };

    for file_id in source_root.iter() {
        if let Some(runnables) = db.file_runnables(file_id) {
            let mut patch = patch().lock().unwrap();
            let mut mutator = ItemMutator::new(RefNode::Crate(& mut res), &mut patch);
            mutator.append(AppendItem::Module(runnables));
        }
    }
    
    if !res.modules.is_empty() {
        return Some(res);
    }

    None
}

fn file_runnables(db: &dyn RunnableDatabase, file_id: FileId) -> Option<Module> {
    fn store_runnables(
        db: &dyn HirDatabase,
        res: &mut Option<Module>,
        path: &mut MutalPath,
        patch: &mut Patch,
        runnables: &[Runnable],
    ) {
        let mut diff_point = find_diff_point(path);

        // If result runnable view is empty, then initialize it's root node
        if let Some(ref point) = diff_point {
            if point.0 == 0 {
                let mut first = path.first_mut().unwrap();

                // TODO: why it is can be Optional
                let name = if let Some(name) = first.origin.name(db) {
                    name.to_string()
                } else {
                    "UNKNOW_MOD_NAME".to_string()
                };

                let item = Module {
                    id: uuid::Uuid::new_v4().as_u128(),
                    name,
                    location: first.origin,
                    content: Default::default(),
                };

                res.replace(item.clone());
                          
                let mut mutator = ItemMutator::new(RefNode::Module(res.as_mut().unwrap()), patch);
                mutator.append(AppendItem::Module(item));
                
                first.accord = Some(res.as_mut().unwrap());

                if path.len() == 1 {
                    diff_point = None;
                } else {
                    diff_point = Some(DifferencePoint(1));
                }
            }
        }

        if let Some(ref dvg_point) = diff_point {
            syn_branches(db, path, dvg_point, patch);
        }

        unsafe {
            let module = &mut (*path.last_mut().unwrap().accord.unwrap());
            let mut mutator = ItemMutator::new(RefNode::Module(module), patch);
            let iter = runnables.into_iter().map(|item| {
                let i = match item {
                    Runnable::Function(func) => func,
                    Runnable::Doctest(_) => todo!(),
                };
                AppendItem::Function(i.clone())
            });
            mutator.append_many(iter);
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

    let mut patch = self::patch().lock().unwrap();
    let mut res = None;

    visit_file_defs_with_path(
        db,
        &sema,
        file_id,
        |db: &dyn RunnableDatabase,
         sema: &Semantics,
         path: &mut MutalPath,
         def: Either<hir::ModuleDef, hir::Impl>| {
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
                store_runnables(db.upcast(), &mut res, path, &mut patch, &runnables);
            }
        },
    );

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
    
    res
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
fn runnable_mod_outline_definition(sema: &Semantics, def: hir::Module) -> Option<Content> {
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
fn has_doctest<AtrOwner: HasAttrs>(
    db: &dyn HirDatabase,
    attrs_onwer: AtrOwner,
) -> Option<Runnable> {
    if !is_contains_runnable_in_doc(&*attrs_onwer.attrs(db)) {
        return None;
    }

    Some(Runnable::Doctest(Doctest { id: todo!(), location: todo!() }))
}

/// Checks if a [hir::Function] is runnable and if it is, then construct [Runnable] from it
fn runnable_fn(sema: &Semantics, def: hir::Function) -> Option<Runnable> {
    let func = def.source(sema.db)?;
    let name_string = def.name(sema.db).to_string();

    let root = def.module(sema.db).krate().root_module(sema.db);

    let kind = if name_string == "main" && def.module(sema.db) == root {
        Some(RunnableFuncKind::Bin)
    } else {
        if extract_test_related_attribute(&func.value).is_some() {
            Some(RunnableFuncKind::Test)
        } else if func.value.has_atom_attr("bench") {
            Some(RunnableFuncKind::Bench)
        } else {
            None
        }
    };

    if let Some(kind) = kind {
        // TODO: func id 
        Some(Runnable::Function(RunnableFunc { id: 1, name: name_string, kind, location: def, }))
    } else {
        None
    }
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
    fn_def
        .attrs()
        .find_map(|attr| attr.path()?.syntax().text().to_string().contains("test").then(|| attr))
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
