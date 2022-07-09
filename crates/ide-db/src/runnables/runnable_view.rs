use hir::Function;
use base_db::FileId;
use hir_def::item_tree::Mod;
use syntax::TextRange;
use serde::{Deserialize, Serialize};

pub type Id = u128;

/// Defines the kind of [RunnableFunc]
#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
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
#[derive(Serialize)]
pub struct Doctest {
    pub id: Id,
    #[serde(skip_serializing)]
    pub location: DoctestLocation,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub struct RunnableFunc {
    pub id: Id,
    pub name: String,
    pub kind: RunnableFuncKind,
    #[serde(skip_serializing)]
    pub location: Function,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub enum Runnable {
    Function(RunnableFunc),
    Doctest(Doctest),
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub struct MacroCall {
    pub id: Id,
    call: (),
    pub content: Vec<Content>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub struct Module {
    pub id: Id,
    pub name: String,
    #[serde(skip_serializing)]
    pub location: hir::Module,
    pub content: Vec<Content>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub enum Node {
    MacroCall(MacroCall),
    Module(Module),
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub enum Content {
    Node(Node),
    Leaf(Runnable),
}

/// We can think about that tree as of a representation a partial view from AST.
/// The main purpose why we need a partial view is that reduce the
/// time to traverse a full tree.
/// That is, this is part of the original tree containing the runnables and branches to them.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Crate {
    pub id: Id,
    pub name: String,
    // location: ,
    pub modules: Vec<Module>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Session {
    pub crates: Vec<Crate>, 
}

pub enum DefKey {
    Crate(hir::Crate),
    Module(hir::Module),
    Function(hir::Function),
}

impl<'a> From<hir::Module> for DefKey {
    fn from(i: hir::Module) -> Self {
        DefKey::Module(i)
    }
}

impl<'a> From<Function> for DefKey {
    fn from(i: Function) -> Self {
        DefKey::Function(i)
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum IterItem<'a> {
    Session(&'a Session),
    Crate(&'a Crate),
    Module(&'a Module),
    // TODO
    MacroCall(&'a MacroCall),
    RunnableFunc(&'a RunnableFunc),
    Doctest(&'a Doctest),
} 

pub fn find_by_def<'tree, Key>(tree_item: IterItem<'tree>, key: Key) -> Option<IterItem<'tree>>
where
    Key: Into<DefKey>,
{
    let def = key.into();

    let mut ret = None;
    dfs(tree_item, |item| match (&def, &item) {
        (DefKey::Function(key), IterItem::RunnableFunc(func)) => {
            if func.location == *key {
                ret = Some(item);
                return true;
            }
            false
        }
        (DefKey::Module(key), IterItem::Module(module)) => {
            if module.location == *key {
                ret = Some(item);
                return true;
            }
            false
        }
        _ => false,
    });
    ret
}

// Just DFS algorithm, that accepts tree root and handler function.
// Handler function return false for continue crawling or true for stop it.
fn dfs<'tree>(root: IterItem<'tree>, mut handler: impl FnMut(IterItem<'tree>) -> bool) {
    let mut buff = vec![root];
    while let Some(item) = buff.pop() {
        match item {
            IterItem::Session(session) => buff.extend(session.crates.iter().map(IterItem::Crate)),
            IterItem::Crate(krate) => buff.extend(krate.modules.iter().map(IterItem::Module)),
            IterItem::Module(m) => {
                let iter = m.content.iter().map(|item| match item {
                    Content::Node(node) => match node {
                        Node::MacroCall(_) => todo!(),
                        Node::Module(module) => IterItem::Module(module),
                    },
                    Content::Leaf(leaf) => match leaf {
                        Runnable::Function(func) => IterItem::RunnableFunc(func),
                        Runnable::Doctest(doctest) => IterItem::Doctest(doctest),
                    },
                });
                buff.extend(iter);
            },
            // IterItem::MacroCall(mc) => buff.extend(mc.content.iter()),
            _ => {},
        }
        if handler(item) {
            break;
        }
    }
}

// Returns an iterator over the contents of a file.
// Note: not including the root of the file.
pub fn flatten_content<'tree>(root: IterItem<'tree>) -> Vec<IterItem<'tree>> {
    let mut res = Vec::new();
    dfs(root.clone(), |item| {
        if item != root {
            match item {
                IterItem::Module(_) 
                | IterItem::RunnableFunc(_) 
                | IterItem::Doctest(_) 
                | IterItem::MacroCall(_) => {
                    res.push(item);
                },
                _ => {},
            }    
        }
        false
    });
    res
}

pub fn find_by_id(root: IterItem, id: Id) -> Option<IterItem> {
    let mut res = None;
    dfs(root, |item| {
        let node_id = match item {
            IterItem::Crate(krate) => krate.id,
            IterItem::MacroCall(macrocall) => macrocall.id,
            IterItem::Module(module) => module.id,
            IterItem::Session(_) => 0,
            IterItem::RunnableFunc(func) => func.id,
            IterItem::Doctest(doctest) => doctest.id,
        };
        if node_id == id {
            res = Some(item);
            return true;
        }

        false
    });
    res
}