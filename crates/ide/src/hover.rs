mod render;

#[cfg(test)]
mod tests;

use std::iter;

use either::Either;
use hir::{HasSource, Semantics};
use ide_db::{
    base_db::{FileRange, Upcast},
    defs::Definition,
    helpers::{pick_best_token, FamousDefs},
    runnables::{RunnableDatabase, Content},
    RootDatabase,
};
use itertools::Itertools;
use syntax::{ast, match_ast, AstNode, SyntaxKind::*, SyntaxNode, SyntaxToken, T};

use crate::{
    display::TryToNav, doc_links::token_as_doc_comment, markup::Markup, FileId, FilePosition,
    NavigationTarget, RangeInfo, Runnable,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverConfig {
    pub links_in_hover: bool,
    pub documentation: Option<HoverDocFormat>,
}

impl HoverConfig {
    fn markdown(&self) -> bool {
        matches!(self.documentation, Some(HoverDocFormat::Markdown))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoverDocFormat {
    Markdown,
    PlainText,
}

#[derive(Debug, Clone)]
pub enum HoverAction {
    Runnable(Runnable),
    Implementation(FilePosition),
    Reference(FilePosition),
    GoToType(Vec<HoverGotoTypeData>),
}

impl HoverAction {
    fn goto_type_from_targets(db: &RootDatabase, targets: Vec<hir::ModuleDef>) -> Self {
        let targets = targets
            .into_iter()
            .filter_map(|it| {
                Some(HoverGotoTypeData {
                    mod_path: render::path(
                        db,
                        it.module(db)?,
                        it.name(db).map(|name| name.to_string()),
                    ),
                    nav: it.try_to_nav(db)?,
                })
            })
            .collect();
        HoverAction::GoToType(targets)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HoverGotoTypeData {
    pub mod_path: String,
    pub nav: NavigationTarget,
}

/// Contains the results when hovering over an item
#[derive(Debug, Default)]
pub struct HoverResult {
    pub markup: Markup,
    pub actions: Vec<HoverAction>,
}

// Feature: Hover
//
// Shows additional information, like the type of an expression or the documentation for a definition when "focusing" code.
// Focusing is usually hovering with a mouse, but can also be triggered with a shortcut.
//
// image::https://user-images.githubusercontent.com/48062697/113020658-b5f98b80-917a-11eb-9f88-3dbc27320c95.gif[]
pub(crate) fn hover(
    db: &RootDatabase,
    FileRange { file_id, range }: FileRange,
    config: &HoverConfig,
) -> Option<RangeInfo<HoverResult>> {
    let sema = &hir::Semantics::new(db);
    let file = sema.parse(file_id).syntax().clone();

    if !range.is_empty() {
        return hover_ranged(db, &file, range, sema, config);
    }
    let offset = range.start();

    let original_token = pick_best_token(file.token_at_offset(offset), |kind| match kind {
        IDENT | INT_NUMBER | LIFETIME_IDENT | T![self] | T![super] | T![crate] => 3,
        T!['('] | T![')'] => 2,
        kind if kind.is_trivia() => 0,
        _ => 1,
    })?;

    if let Some(doc_comment) = token_as_doc_comment(&original_token) {
        cov_mark::hit!(no_highlight_on_comment_hover);
        return doc_comment.get_definition_with_descend_at(sema, offset, |def, node, range| {
            let res = hover_for_definition(db, sema, file_id, def, &node, config)?;
            Some(RangeInfo::new(range, res))
        });
    }

    let descended = sema.descend_into_macros(original_token.clone());

    // FIXME: Definition should include known lints and the like instead of having this special case here
    if let Some(res) = descended.iter().find_map(|token| {
        let attr = token.ancestors().find_map(ast::Attr::cast)?;
        render::try_for_lint(&attr, token)
    }) {
        return Some(RangeInfo::new(original_token.text_range(), res));
    }

    let result = descended
        .iter()
        .filter_map(|token| {
            let node = token.parent()?;
            let defs = Definition::from_token(sema, token);
            Some(defs.into_iter().zip(iter::once(node).cycle()))
        })
        .flatten()
        .unique_by(|&(def, _)| def)
        .filter_map(|(def, node)| hover_for_definition(db, sema, file_id, def, &node, config))
        .reduce(|mut acc, HoverResult { markup, actions }| {
            acc.actions.extend(actions);
            acc.markup = Markup::from(format!("{}\n---\n{}", acc.markup, markup));
            acc
        });
    if result.is_none() {
        // fallbacks, show keywords or types
        if let Some(res) = render::keyword(db, sema, config, &original_token) {
            return Some(RangeInfo::new(original_token.text_range(), res));
        }
        if let res @ Some(_) =
            descended.iter().find_map(|token| hover_type_fallback(db, sema, config, token))
        {
            return res;
        }
    }
    result.map(|res| RangeInfo::new(original_token.text_range(), res))
}

pub(crate) fn hover_for_definition(
    db: &RootDatabase,
    sema: &Semantics,
    file_id: FileId,
    definition: Definition,
    node: &SyntaxNode,
    config: &HoverConfig,
) -> Option<HoverResult> {
    let famous_defs = match &definition {
        Definition::ModuleDef(hir::ModuleDef::BuiltinType(_)) => {
            Some(FamousDefs(sema, sema.scope(node).krate()))
        }
        _ => None,
    };
    if let Some(markup) = render::definition(db, definition, famous_defs.as_ref(), config) {
        let mut res = HoverResult::default();
        res.markup = render::process_markup(db, definition, &markup, config);
        if let Some(action) = show_implementations_action(db, definition) {
            res.actions.push(action);
        }

        if let Some(action) = show_fn_references_action(db, definition) {
            res.actions.push(action);
        }

        if let Some(action) = runnable_action(db, sema, definition, file_id) {
            res.actions.push(action);
        }

        if let Some(action) = goto_type_action_for_def(db, definition) {
            res.actions.push(action);
        }
        return Some(res);
    }
    None
}

fn hover_ranged(
    db: &RootDatabase,
    file: &SyntaxNode,
    range: syntax::TextRange,
    sema: &Semantics,
    config: &HoverConfig,
) -> Option<RangeInfo<HoverResult>> {
    let expr_or_pat = file.covering_element(range).ancestors().find_map(|it| {
        match_ast! {
            match it {
                ast::Expr(expr) => Some(Either::Left(expr)),
                ast::Pat(pat) => Some(Either::Right(pat)),
                _ => None,
            }
        }
    })?;
    let res = match &expr_or_pat {
        Either::Left(ast::Expr::TryExpr(try_expr)) => render::try_expr(db, sema, config, try_expr),
        Either::Left(ast::Expr::PrefixExpr(prefix_expr))
            if prefix_expr.op_kind() == Some(ast::UnaryOp::Deref) =>
        {
            render::deref_expr(db, sema, config, prefix_expr)
        }
        _ => None,
    };
    let res = res.or_else(|| render::type_info(db, sema, config, &expr_or_pat));
    res.map(|it| {
        let range = match expr_or_pat {
            Either::Left(it) => it.syntax().text_range(),
            Either::Right(it) => it.syntax().text_range(),
        };
        RangeInfo::new(range, it)
    })
}

fn hover_type_fallback(
    db: &RootDatabase,
    sema: &Semantics,
    config: &HoverConfig,
    token: &SyntaxToken,
) -> Option<RangeInfo<HoverResult>> {
    let node = token
        .ancestors()
        .take_while(|it| !ast::Item::can_cast(it.kind()))
        .find(|n| ast::Expr::can_cast(n.kind()) || ast::Pat::can_cast(n.kind()))?;

    let expr_or_pat = match_ast! {
        match node {
            ast::Expr(it) => Either::Left(it),
            ast::Pat(it) => Either::Right(it),
            // If this node is a MACRO_CALL, it means that `descend_into_macros_many` failed to resolve.
            // (e.g expanding a builtin macro). So we give up here.
            ast::MacroCall(_it) => return None,
            _ => return None,
        }
    };

    let res = render::type_info(db, sema, config, &expr_or_pat)?;
    let range = sema.original_range(&node).range;
    Some(RangeInfo::new(range, res))
}

fn show_implementations_action(db: &RootDatabase, def: Definition) -> Option<HoverAction> {
    fn to_action(nav_target: NavigationTarget) -> HoverAction {
        HoverAction::Implementation(FilePosition {
            file_id: nav_target.file_id,
            offset: nav_target.focus_or_full_range().start(),
        })
    }

    let adt = match def {
        Definition::ModuleDef(hir::ModuleDef::Trait(it)) => {
            return it.try_to_nav(db).map(to_action)
        }
        Definition::ModuleDef(hir::ModuleDef::Adt(it)) => Some(it),
        Definition::SelfType(it) => it.self_ty(db).as_adt(),
        _ => None,
    }?;
    adt.try_to_nav(db).map(to_action)
}

fn show_fn_references_action(db: &RootDatabase, def: Definition) -> Option<HoverAction> {
    match def {
        Definition::ModuleDef(hir::ModuleDef::Function(it)) => {
            it.try_to_nav(db).map(|nav_target| {
                HoverAction::Reference(FilePosition {
                    file_id: nav_target.file_id,
                    offset: nav_target.focus_or_full_range().start(),
                })
            })
        }
        _ => None,
    }
}

fn runnable_action(
    db: &RootDatabase,
    sema: &hir::Semantics,
    def: Definition,
    file_id: FileId,
) -> Option<HoverAction> {
    use ide_db::runnables::*;

    let rnb: &dyn RunnableDatabase = db.upcast();

    let rnbls = rnb.file_runnables(file_id);
    if let (Some(module), Definition::ModuleDef(mod_def)) = (rnbls, def) {
        match mod_def {
            hir::ModuleDef::Module(it) => {
                return find_by_def(IterItem::Module(&module), it)
                        .map(|i| { 
                            let r = match i {
                                IterItem::Module(module) => Content::Node(Node::Module(module.clone())),
                                _ => unreachable!(),
                            };
                            HoverAction::Runnable(self::Runnable::from_db_repr(db, sema, &r))
                        });
            }
            hir::ModuleDef::Function(it) => {
                let src = it.source(sema.db)?;
                if src.file_id != file_id.into() {
                    cov_mark::hit!(hover_macro_generated_struct_fn_doc_comment);
                    cov_mark::hit!(hover_macro_generated_struct_fn_doc_attr);
                    return None;
                }

                return find_by_def(IterItem::Module(&module), it)
                        .map(|i| { 
                            let r = match i {
                                IterItem::RunnableFunc(func) => Content::Leaf(Runnable::Function(func.clone())),
                                _ => unreachable!(),
                            };
                            HoverAction::Runnable(self::Runnable::from_db_repr(db, sema, &r))
                        });
            }
            _ => {}
        }
    }

    None
}

fn goto_type_action_for_def(db: &RootDatabase, def: Definition) -> Option<HoverAction> {
    let mut targets: Vec<hir::ModuleDef> = Vec::new();
    let mut push_new_def = |item: hir::ModuleDef| {
        if !targets.contains(&item) {
            targets.push(item);
        }
    };

    if let Definition::GenericParam(hir::GenericParam::TypeParam(it)) = def {
        it.trait_bounds(db).into_iter().for_each(|it| push_new_def(it.into()));
    } else {
        let ty = match def {
            Definition::Local(it) => it.ty(db),
            Definition::GenericParam(hir::GenericParam::ConstParam(it)) => it.ty(db),
            Definition::Field(field) => field.ty(db),
            _ => return None,
        };

        walk_and_push_ty(db, &ty, &mut push_new_def);
    }

    Some(HoverAction::goto_type_from_targets(db, targets))
}

fn walk_and_push_ty(
    db: &RootDatabase,
    ty: &hir::Type,
    push_new_def: &mut dyn FnMut(hir::ModuleDef),
) {
    ty.walk(db, |t| {
        if let Some(adt) = t.as_adt() {
            push_new_def(adt.into());
        } else if let Some(trait_) = t.as_dyn_trait() {
            push_new_def(trait_.into());
        } else if let Some(traits) = t.as_impl_traits(db) {
            traits.into_iter().for_each(|it| push_new_def(it.into()));
        } else if let Some(trait_) = t.as_associated_type_parent_trait(db) {
            push_new_def(trait_.into());
        }
    });
}
