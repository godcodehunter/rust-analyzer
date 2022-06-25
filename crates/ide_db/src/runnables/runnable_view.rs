use hir::Function;
use base_db::FileId;
use syntax::TextRange;
use serde::{Deserialize, Serialize};

pub type Id = usize;

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
    pub content: Vec<RunnableView>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub struct Module {
    pub id: Id,
    pub name: String,
    #[serde(skip_serializing)]
    pub location: hir::Module,
    pub content: Vec<RunnableView>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
pub enum Node {
    MacroCall(MacroCall),
    Module(Module),
}

/// We can think about that tree as of a representation a partial view from AST.
/// The main purpose why we need a partial view is that reduce the
/// time to traverse a full tree.
/// That is, this is part of the original tree containing the runnables and branches to them.
#[derive(PartialEq, Eq, Debug, Clone)]
#[derive(Serialize)]
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
    where
        Key: Into<DefKey<'a>>,
    {
        let def = &key.into();

        let mut ret = None;
        Self::dfs(self, |it| match (def, it) {
            (DefKey::Function(key), RunnableView::Leaf(Runnable::Function(func))) => {
                if func.location == **key {
                    ret = Some(it);
                    return true;
                }
                false
            }
            (DefKey::Module(key), RunnableView::Node(Node::Module(node))) => {
                if node.location == **key {
                    ret = Some(it);
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
    fn dfs<'a>(root: &'a RunnableView, mut handler: impl FnMut(&'a RunnableView) -> bool) {
        let mut buff = vec![root];
        while let Some(item) = buff.pop() {
            match item {
                RunnableView::Node(Node::Module(m)) => buff.extend(m.content.iter()),
                RunnableView::Node(Node::MacroCall(mc)) => buff.extend(mc.content.iter()),
                _ => {}
            }
            if handler(item) {
                break;
            }
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

    pub fn get_rnbl_by_id(&self, id: Id) -> Option<&RunnableView> {
        todo!()
    }
}