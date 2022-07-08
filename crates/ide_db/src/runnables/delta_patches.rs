use super::{Node, Module, Id, runnable_view::{MacroCall, Content, RunnableFuncKind, RunnableFunc, Crate, self, Runnable}};
use serde::{Deserialize, Serialize};

pub trait Mutator<Id, AppendItem, Changes> {
    fn delete(&mut self, item_id: Id);
    // fn delete_all(&mut self) {}
    // fn delete_many((&mut self, items: impl IntoIterator<Item = Id>) {}
    fn append(&mut self, item: AppendItem);
    fn append_many(&mut self, items: impl IntoIterator<Item = AppendItem>) {
        for item in items {
            self.append(item);
        }
    }
    fn update(&mut self, update: Changes);
}

trait ChangeObserver<Id, AppendItem, Changes> {
    /// Removes child node with id `item_id` from parent node with id `target_id`
    fn delete(&mut self, target_id: Id, item_id: Id);
    /// Creates a child node `Item` for the node with id `target_id`
    fn append(&mut self, target_id: Id, item: AppendItem);
    /// Applies the update `Update` to the node with id `target_id`
    fn update(&mut self, target_id: Id, update: Changes);
}

#[derive(Debug, Default, Clone)]
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

    pub fn is_empty(&self) -> bool {
        self.delete.is_empty() 
        && self.append.is_empty() 
        && self.update.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Delete {
    pub target_id: Id,
    pub item_id: Id,
}   

#[derive(Debug, Clone)]
pub enum AppendItem {
    Crate(runnable_view::Crate),
    Module(Module),
    Function(RunnableFunc),
}

#[derive(Debug, Clone)]
pub struct Append {
    pub target_id: Id,
    pub item: AppendItem,
}

#[derive(Debug, Clone)]
pub struct Update {
    target_id: Id,
    changes: Changes, 
}

#[derive(Debug, Clone)]
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

pub enum RefNode<'a> {
    Crate(&'a mut Crate),
    Module(&'a mut Module),
    MacroCall(&'a mut MacroCall),
}

pub struct ItemMutator<'a, 'b> {
    /// Represent target node or if not presented, assume that root node 
    target: Option<RefNode<'a>>,
    patch: &'b mut Patch,
}

impl<'a, 'b> ItemMutator<'a, 'b> {
    pub fn new(node: Option<RefNode<'a>>, patch: &'b mut Patch) -> Self { 
        Self { target: node, patch } 
    }

    /// Return node id or for root `0`
    fn target_id(&self) -> Id {
        self.target.as_ref().map_or(0, Self::node_id)
    }

    fn node_id(node: &RefNode<'_>) -> Id {
        match node {
            RefNode::Crate(krate) => krate.id,
            RefNode::MacroCall(macrocall) => macrocall.id,
            RefNode::Module(module) => module.id,
        }
    }
}

impl<'a, 'b> Mutator<Id, AppendItem, Changes> for ItemMutator<'a, 'b> {
    fn delete(&mut self, id: Id) {
        if let Some(ref mut node) = self.target {
            match node {
                RefNode::Crate(krate) => {
                    let index = krate.modules.iter()
                        .position(|item| item.id == id)
                        .unwrap();
                },
                RefNode::MacroCall(macrocall) => {
                    let index = macrocall.content.iter()
                        .position(|item| {
                            match item {
                                Content::Node(node) => todo!(),
                                Content::Leaf(leaf) => todo!(),
                            }
                        })
                        .unwrap();
                    macrocall.content.remove(index);
                },
                RefNode::Module(module) => {
                    let index = module.content.iter()
                        .position(|item| {
                            match item {
                                Content::Node(node) => todo!(),
                                Content::Leaf(leaf) => todo!(),
                            }
                        })
                        .unwrap();
                    module.content.remove(index);
                },
            }
        }
        
        self.patch.delete(self.target_id(), id);
    }

    fn append(&mut self, item: AppendItem) {
        if let Some(ref mut node) = self.target {
            match node {
                RefNode::Crate(krate) => {
                    if let AppendItem::Module(ref module) = item {
                        krate.modules.push(module.clone());
                    }
                    todo!()
                },
                RefNode::MacroCall(macrocall) => {
                    match item {
                        AppendItem::Crate(krate) => {
                            // macrocall.content.push(Content::Node());
                            todo!()
                        },
                        AppendItem::Module(_) => {
                            // macrocall.content.push(item.clone());
                            todo!()
                        },
                        AppendItem::Function(_) => todo!(),
                    }
                },
                RefNode::Module(module) => {
                    let i = match item {
                        AppendItem::Crate(_) => todo!(),
                        AppendItem::Module(ref module) => Content::Node(Node::Module(module.clone())),
                        AppendItem::Function(ref func) => Content::Leaf(Runnable::Function(func.clone())),
                    };

                    module.content.push(i);
                },
            }
        }

        self.patch.append(self.target_id(), item);
    }

    fn update(&mut self, update: Changes) {
        if let Some(ref mut node) = self.target {
            match node {
                RefNode::Crate(krate) => {
                    todo!();
                },
                RefNode::MacroCall(macrocall) => {
                    if let Changes::MacroCall {  } = update {
                        todo!();
                    }

                    panic!("mismatched update type");
                },
                RefNode::Module(module) => {
                    if let Changes::Module { ref name, ref location} = update {
                        if let Some(name) = name {
                            // module.name = name.clone();
                            todo!()
                        }
                        if let Some(location) = location {
                            // module.location = location;
                            todo!()
                        }
                    }
                    panic!("mismatched update type");
                },
            }
        }

        // self.patch.update(self.target_id(), update);
        todo!()
    }
}

impl ChangeObserver<Id, AppendItem, Changes> for Patch {
    fn delete(&mut self, target_id: Id, item_id: Id) {
        self.delete.push(Delete {
            target_id,
            item_id,
        });
    }

    fn append(&mut self, target_id: Id, item: AppendItem) {
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
