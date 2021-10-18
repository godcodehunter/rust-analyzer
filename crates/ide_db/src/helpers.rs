//! A module with ide helpers for high-level ide features.
pub mod famous_defs;
pub mod generated_lints;
pub mod import_assets;
pub mod insert_use;
pub mod merge_imports;
pub mod node_ext;
pub mod rust_doc;

use std::collections::VecDeque;

use base_db::FileId;
use either::Either;
use hir::{ItemInNs, MacroDef, ModuleDef, Name, Semantics, db::HirDatabase};
use syntax::{
    ast::{self, make, HasLoopBody},
    AstNode, Direction, SyntaxElement, SyntaxKind, SyntaxToken, TokenAtOffset, WalkEvent, T,
};

pub use self::famous_defs::FamousDefs;

pub fn item_name(db: &dyn HirDatabase, item: ItemInNs) -> Option<Name> {
    match item {
        ItemInNs::Types(module_def_id) => ModuleDef::from(module_def_id).name(db),
        ItemInNs::Values(module_def_id) => ModuleDef::from(module_def_id).name(db),
        ItemInNs::Macros(macro_def_id) => MacroDef::from(macro_def_id).name(db),
    }
}

/// Resolves the path at the cursor token as a derive macro if it inside a token tree of a derive attribute.
pub fn try_resolve_derive_input_at(
    sema: &hir::Semantics,
    derive_attr: &ast::Attr,
    cursor: &SyntaxToken,
) -> Option<MacroDef> {
    use itertools::Itertools;
    if cursor.kind() != T![ident] {
        return None;
    }
    let tt = match derive_attr.as_simple_call() {
        Some((name, tt))
            if name == "derive" && tt.syntax().text_range().contains_range(cursor.text_range()) =>
        {
            tt
        }
        _ => return None,
    };
    let tokens: Vec<_> = cursor
        .siblings_with_tokens(Direction::Prev)
        .flat_map(SyntaxElement::into_token)
        .take_while(|tok| tok.kind() != T!['('] && tok.kind() != T![,])
        .collect();
    let path = ast::Path::parse(&tokens.into_iter().rev().join("")).ok()?;
    match sema.scope(tt.syntax()).speculative_resolve(&path) {
        Some(hir::PathResolution::Macro(makro)) if makro.kind() == hir::MacroKind::Derive => {
            Some(makro)
        }
        _ => None,
    }
}

/// Picks the token with the highest rank returned by the passed in function.
pub fn pick_best_token(
    tokens: TokenAtOffset<SyntaxToken>,
    f: impl Fn(SyntaxKind) -> usize,
) -> Option<SyntaxToken> {
    tokens.max_by_key(move |t| f(t.kind()))
}

/// Converts the mod path struct into its ast representation.
pub fn mod_path_to_ast(path: &hir::ModPath) -> ast::Path {
    let _p = profile::span("mod_path_to_ast");

    let mut segments = Vec::new();
    let mut is_abs = false;
    match path.kind {
        hir::PathKind::Plain => {}
        hir::PathKind::Super(0) => segments.push(make::path_segment_self()),
        hir::PathKind::Super(n) => segments.extend((0..n).map(|_| make::path_segment_super())),
        hir::PathKind::DollarCrate(_) | hir::PathKind::Crate => {
            segments.push(make::path_segment_crate())
        }
        hir::PathKind::Abs => is_abs = true,
    }

    segments.extend(
        path.segments()
            .iter()
            .map(|segment| make::path_segment(make::name_ref(&segment.to_string()))),
    );
    make::path_from_segments(segments, is_abs)
}

/// Iterates all `ModuleDef`s and `Impl` blocks of the given file.
pub fn visit_file_defs(
    sema: &Semantics,
    file_id: FileId,
    cb: &mut dyn FnMut(Either<hir::ModuleDef, hir::Impl>),
) {
    let db = sema.db;
    let module = match sema.to_module_def(file_id) {
        Some(it) => it,
        None => return,
    };
    let mut defs: VecDeque<_> = module.declarations(db).into();
    while let Some(def) = defs.pop_front() {
        if let ModuleDef::Module(submodule) = def {
            if let hir::ModuleSource::Module(_) = submodule.definition_source(db).value {
                defs.extend(submodule.declarations(db));
                submodule.impl_defs(db).into_iter().for_each(|impl_| cb(Either::Right(impl_)));
            }
        }
        cb(Either::Left(def));
    }
    module.impl_defs(db).into_iter().for_each(|impl_| cb(Either::Right(impl_)));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnippetCap {
    _private: (),
}

impl SnippetCap {
    pub const fn new(allow_snippets: bool) -> Option<SnippetCap> {
        if allow_snippets {
            Some(SnippetCap { _private: () })
        } else {
            None
        }
    }
}

/// Calls `cb` on each expression inside `expr` that is at "tail position".
/// Does not walk into `break` or `return` expressions.
/// Note that modifying the tree while iterating it will cause undefined iteration which might
/// potentially results in an out of bounds panic.
pub fn for_each_tail_expr(expr: &ast::Expr, cb: &mut dyn FnMut(&ast::Expr)) {
    match expr {
        ast::Expr::BlockExpr(b) => {
            match b.modifier() {
                Some(
                    ast::BlockModifier::Async(_)
                    | ast::BlockModifier::Try(_)
                    | ast::BlockModifier::Const(_),
                ) => return cb(expr),

                Some(ast::BlockModifier::Label(label)) => {
                    for_each_break_expr(Some(label), b.stmt_list(), &mut |b| {
                        cb(&ast::Expr::BreakExpr(b))
                    });
                }
                Some(ast::BlockModifier::Unsafe(_)) => (),
                None => (),
            }
            if let Some(stmt_list) = b.stmt_list() {
                if let Some(e) = stmt_list.tail_expr() {
                    for_each_tail_expr(&e, cb);
                }
            }
        }
        ast::Expr::IfExpr(if_) => {
            let mut if_ = if_.clone();
            loop {
                if let Some(block) = if_.then_branch() {
                    for_each_tail_expr(&ast::Expr::BlockExpr(block), cb);
                }
                match if_.else_branch() {
                    Some(ast::ElseBranch::IfExpr(it)) => if_ = it,
                    Some(ast::ElseBranch::Block(block)) => {
                        for_each_tail_expr(&ast::Expr::BlockExpr(block), cb);
                        break;
                    }
                    None => break,
                }
            }
        }
        ast::Expr::LoopExpr(l) => {
            for_each_break_expr(l.label(), l.loop_body().and_then(|it| it.stmt_list()), &mut |b| {
                cb(&ast::Expr::BreakExpr(b))
            })
        }
        ast::Expr::MatchExpr(m) => {
            if let Some(arms) = m.match_arm_list() {
                arms.arms().filter_map(|arm| arm.expr()).for_each(|e| for_each_tail_expr(&e, cb));
            }
        }
        ast::Expr::ArrayExpr(_)
        | ast::Expr::AwaitExpr(_)
        | ast::Expr::BinExpr(_)
        | ast::Expr::BoxExpr(_)
        | ast::Expr::BreakExpr(_)
        | ast::Expr::CallExpr(_)
        | ast::Expr::CastExpr(_)
        | ast::Expr::ClosureExpr(_)
        | ast::Expr::ContinueExpr(_)
        | ast::Expr::FieldExpr(_)
        | ast::Expr::ForExpr(_)
        | ast::Expr::IndexExpr(_)
        | ast::Expr::Literal(_)
        | ast::Expr::MacroCall(_)
        | ast::Expr::MacroStmts(_)
        | ast::Expr::MethodCallExpr(_)
        | ast::Expr::ParenExpr(_)
        | ast::Expr::PathExpr(_)
        | ast::Expr::PrefixExpr(_)
        | ast::Expr::RangeExpr(_)
        | ast::Expr::RecordExpr(_)
        | ast::Expr::RefExpr(_)
        | ast::Expr::ReturnExpr(_)
        | ast::Expr::TryExpr(_)
        | ast::Expr::TupleExpr(_)
        | ast::Expr::WhileExpr(_)
        | ast::Expr::YieldExpr(_) => cb(expr),
    }
}

/// Calls `cb` on each break expr inside of `body` that is applicable for the given label.
pub fn for_each_break_expr(
    label: Option<ast::Label>,
    body: Option<ast::StmtList>,
    cb: &mut dyn FnMut(ast::BreakExpr),
) {
    let label = label.and_then(|lbl| lbl.lifetime());
    let mut depth = 0;
    if let Some(b) = body {
        let preorder = &mut b.syntax().preorder();
        let ev_as_expr = |ev| match ev {
            WalkEvent::Enter(it) => Some(WalkEvent::Enter(ast::Expr::cast(it)?)),
            WalkEvent::Leave(it) => Some(WalkEvent::Leave(ast::Expr::cast(it)?)),
        };
        let eq_label = |lt: Option<ast::Lifetime>| {
            lt.zip(label.as_ref()).map_or(false, |(lt, lbl)| lt.text() == lbl.text())
        };
        while let Some(node) = preorder.find_map(ev_as_expr) {
            match node {
                WalkEvent::Enter(expr) => match expr {
                    ast::Expr::LoopExpr(_) | ast::Expr::WhileExpr(_) | ast::Expr::ForExpr(_) => {
                        depth += 1
                    }
                    ast::Expr::BlockExpr(e) if e.label().is_some() => depth += 1,
                    ast::Expr::BreakExpr(b)
                        if (depth == 0 && b.lifetime().is_none()) || eq_label(b.lifetime()) =>
                    {
                        cb(b);
                    }
                    _ => (),
                },
                WalkEvent::Leave(expr) => match expr {
                    ast::Expr::LoopExpr(_) | ast::Expr::WhileExpr(_) | ast::Expr::ForExpr(_) => {
                        depth -= 1
                    }
                    ast::Expr::BlockExpr(e) if e.label().is_some() => depth -= 1,
                    _ => (),
                },
            }
        }
    }
}
