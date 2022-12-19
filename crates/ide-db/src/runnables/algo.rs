use crate::RootDatabase;

use super::*;

pub struct Bijection {
    pub origin: hir::Module,
    pub accord: Option<*mut Module>,
}

pub type MutalPath = Vec<Bijection>;

/// Represents the point from which paths begin to differ
pub struct DifferencePoint(pub usize);

/// Compares paths and returns [DifferencePoint] if they are not equvalent
pub fn find_diff_point(path: &MutalPath) -> Option<DifferencePoint> {
    for item in path.into_iter().enumerate() {
        if item.1.accord.is_none() {
            return Some(DifferencePoint(item.0));
        }
    }

    None
}

/// Reconstructs [RunnableView] branch and maintains consistency [MutalPath]
/// in the process.
pub fn syn_branches<'path>(
    db: &dyn HirDatabase,
    path: &'path mut MutalPath, 
    dvg_point: &DifferencePoint, 
    patch: &mut Patch,
) {
    let mut iter = path.iter_mut().skip(dvg_point.0 - 1);
    let last_sync = iter.next().unwrap();

    iter.fold(last_sync, |cur: &mut Bijection, next: &mut Bijection| -> &mut Bijection {
        // TODO: why it is can be Optional
        let name = if let Some(name) = next.origin.name(db) {
            name.to_string()
        } else {
            "UNKNOW_MOD_NAME".to_string()
        };
        
        let node = Module { 
            id: uuid::Uuid::new_v4().as_u128().to_string(), 
            name, 
            location: next.origin, 
            content: Default::default(), 
        };
        
        unsafe {
            let mut mutator = ItemMutator::new(RefNode::Module(&mut *cur.accord.unwrap()), patch);
            mutator.append(AppendItem::Module(node));
            if let Content::Node(Node::Module(ref mut m)) = (*cur.accord.unwrap()).content.last_mut().unwrap() {
                next.accord = Some(m);
            }
        }
        next
    });
}

/// Iterates all `ModuleDef`s and `Impl` blocks of the given file,   
pub fn visit_file_defs_with_path(
    db: &dyn HirDatabase,
    file_id: FileId,
    mut callback: impl FnMut(
        &dyn HirDatabase,
        &mut MutalPath,
        Either<hir::ModuleDef, hir::Impl>,
    ),
) {
    fn file_to_def(db: &dyn HirDatabase, file: FileId) -> Option<hir::Module> {
        let _p = profile::span("SourceBinder::to_module_def");
        let mut mods = Vec::new();
        for &crate_id in db.relevant_crates(file).iter() {
            // FIXME: inner items
            let crate_def_map = db.crate_def_map(crate_id);
            mods.extend(
                crate_def_map
                    .modules_for_file(file)
                    .map(|local_id| crate_def_map.module_id(local_id)),
            )
        }
        mods.into_iter().map(hir::Module::from).next()
    }

    // TODO: 
    // sema.to_module_def(file_id)

    let module = match file_to_def(db, file_id) {
        Some(it) => it,
        None => return,
    };

    let mut path = MutalPath::new();

    let declarations = module.declarations(db);
    if declarations.is_empty() {
        return;
    }
    path.push(Bijection { origin: module, accord: None });
    let mut walk_queue: Vec<(hir::Module, Vec<ModuleDef>)> = vec![(module, declarations)];

    while let Some((parent, childrens)) = walk_queue.last_mut() {
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

        callback(db, &mut path, Either::Left(defenition));
        if let ModuleDef::Module(module) = defenition {
            for impl_ in module.impl_defs(db) {
                callback(db, &mut path, Either::Right(impl_))
            }
        }

        if let ModuleDef::Module(module) = defenition {
            if let hir::ModuleSource::Module(_) = module.definition_source(db).value {
                let declartions = module.declarations(db);
                if !declartions.is_empty() {
                    path.push(Bijection { origin: module, accord: None });
                    walk_queue.push((module, declartions));
                }
            }
        }
    }
}