use super::{Node, Module, Id, runnable_view::{RunnableView, RunnableFuncKind}};
use serde::{Deserialize, Serialize};

pub trait Mutator<Id, Item, Changes> {
    fn delete(&mut self, item_id: Id);
    fn append(&mut self, item: Item);
    fn update(&mut self, update: Changes);
}

trait ChangeObserver<Id, Item, Changes> {
    /// Removes child node with id `item_id` from parent node with id `target_id`
    fn delete(&mut self, target_id: Id, item_id: Id);
    /// Creates a child node `Item` for the node with id `target_id`
    fn append(&mut self, target_id: Id, item: Item);
    /// Applies the update `Update` to the node with id `target_id`
    fn update(&mut self, target_id: Id, update: Changes);
}

#[derive(Default)]
pub struct Patch {
    pub id: u64,
    pub delete: Vec<Delete>,
    pub append: Vec<Append>,
    pub update: Vec<Update>,
}

impl Patch {
    /// Used by patch consumer to notify that it has successfully 
    /// consumed the patch
    /// 
    /// NOTE: Shifts the patch identifier and reset patch storages
    pub fn was_consumed(&mut self) {
        self.id += 1;
        self.delete.clear();
        self.append.clear();
        self.update.clear();
    }
}

pub struct Delete {
    pub target_id: Id,
    pub item_id: Id,
}   

pub struct Append {
    pub target_id: Id,
    pub item: RunnableView,
}

pub struct Update {
    target_id: Id,
    changes: Changes, 
}

pub enum Changes {
    RunnableFunc {
        name: Option<String>,
        location: Option<String>,
        kind: Option<RunnableFuncKind>,
    },
    MacroCall {

    },
    Module {
        name: Option<String>,
        location: Option<String>,
    },
    Crate {
        name: Option<String>,
        location: Option<String>,
    },
    Package {
        name: Option<String>,
        location: Option<String>,
    },
}

pub struct ItemMutator<'a, 'b> {
    node: &'a mut Node,
    patch: &'b mut Patch,
}

impl<'a, 'b> ItemMutator<'a, 'b> {
    pub fn new(node: &'a mut Node, patch: &'b mut Patch) -> Self { 
        Self { node, patch } 
    }
}

impl<'a, 'b> Mutator<Id, RunnableView, Changes> for ItemMutator<'a, 'b> {
    fn delete(&mut self, id: Id) {
        // match self.node {
        //     Node::MacroCall(macrocall) => {
        //         let index = macrocall.content.iter()
        //             .position(|item| item.id == id)
        //             .unwrap();
        //         macrocall.content.remove(index);

        //         self.patch.delete(macrocall.id, id);
        //     },
        //     Node::Module(module) => {
        //         let index = module.content.iter()
        //             .position(|item| item.id == id)
        //             .unwrap();
        //         module.content.remove(index);

        //         self.patch.delete(module.id, id);
        //     },
        // }
        todo!()
    }

    fn append(&mut self, item: RunnableView) {
        match self.node {
            Node::MacroCall(macrocall) => {
                macrocall.content.push(item.clone());

                self.patch.append(macrocall.id, item);
            },
            Node::Module(module) => {
                module.content.push(item.clone());

                self.patch.append(module.id, item);
            },
        }
    }

    fn update(&mut self, update: Changes) {
        match self.node {
            Node::MacroCall(macrocall) => {
                if let Changes::MacroCall {  } = update {
                    todo!();

                    self.patch.update(macrocall.id, update);
                }

                panic!("mismatched update type");
            },
            Node::Module(module) => {
                if let Changes::Module { ref name, ref location} = update {
                    if let Some(name) = name {
                        module.name = name.clone();
                    }
                    if let Some(location) = location {
                        // module.location = location;
                        todo!()
                    }

                    self.patch.update(module.id, update);
                }
                panic!("mismatched update type");
            },
        }
    }
}

impl ChangeObserver<Id, RunnableView, Changes> for Patch {
    fn delete(&mut self, target_id: Id, item_id: Id) {
        self.delete.push(Delete {
            target_id,
            item_id,
        });
    }

    fn append(&mut self, target_id: Id, item: RunnableView) {
        self.append.push(Append {
            target_id,
            item,
        });
    }

    fn update(&mut self, target_id: Id, changes: Changes) {
        self.update.push(Update {
            target_id,
            changes,
        });
    }
}

fn apply_patch(tree: &mut RunnableView, patch: &Patch) {

}
