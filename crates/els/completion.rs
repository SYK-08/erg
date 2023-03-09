use serde::Deserialize;
use serde_json::json;
use serde_json::Value;

use erg_common::erg_util::BUILTIN_ERG_MODS;
use erg_common::impl_u8_enum;
use erg_common::python_util::BUILTIN_PYTHON_MODS;
use erg_common::set::Set;
use erg_common::traits::Locational;

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::context::Context;
use erg_compiler::erg_parser::token::TokenKind;
use erg_compiler::hir::Expr;
use erg_compiler::ty::{HasType, ParamTy, Type};
use erg_compiler::varinfo::{AbsLocation, VarInfo};
use TokenKind::*;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, Documentation, MarkedString,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::server::{send, send_log, ELSResult, Server};
use crate::util;

#[derive(Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Local,
    Space,
    LParen,
    Method,
    // Colon, // :, Type ascription or private access `::`
}

impl CompletionKind {
    pub const fn should_be_local(&self) -> bool {
        matches!(self, Self::Local | Self::Space | Self::LParen)
    }

    pub const fn should_be_method(&self) -> bool {
        matches!(self, Self::Method)
    }

    pub const fn _is_lparen(&self) -> bool {
        matches!(self, Self::LParen)
    }
}

fn mark_to_string(mark: MarkedString) -> String {
    match mark {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(ls) => format!("```{}\n{}\n```", ls.language, ls.value),
    }
}

fn markdown_order(block: &str) -> usize {
    if block.starts_with("```") {
        usize::MAX
    } else {
        0
    }
}

impl_u8_enum! { CompletionOrder; i32;
    TypeMatched = -32,
    NameMatched = -8,
    ReturnTypeMatched = -2,
    Normal = 1000000,
    Builtin = 1,
    OtherNamespace = 2,
    Escaped = 32,
    DoubleEscaped = 64,
}

impl CompletionOrder {
    pub const BUILTIN_MOD: char = match char::from_u32(
        CompletionOrder::Normal as u32
            + CompletionOrder::Builtin as u32
            + CompletionOrder::OtherNamespace as u32,
    ) {
        Some(c) => c,
        None => unreachable!(),
    };
}

pub struct CompletionOrderSetter<'b> {
    vi: &'b VarInfo,
    arg_pt: Option<&'b ParamTy>,
    mod_ctx: &'b Context, // for subtype judgement, not for variable lookup
    label: String,
}

impl<'b> CompletionOrderSetter<'b> {
    pub fn new(
        vi: &'b VarInfo,
        arg_pt: Option<&'b ParamTy>,
        mod_ctx: &'b Context,
        label: String,
    ) -> Self {
        Self {
            vi,
            arg_pt,
            mod_ctx,
            label,
        }
    }

    pub fn score(&self) -> i32 {
        let mut orders = vec![CompletionOrder::Normal];
        if self.label.starts_with("__") {
            orders.push(CompletionOrder::DoubleEscaped);
        } else if self.label.starts_with('_') {
            orders.push(CompletionOrder::Escaped);
        }
        if self.vi.kind.is_builtin() {
            orders.push(CompletionOrder::Builtin);
        }
        if self
            .arg_pt
            .map_or(false, |pt| pt.name().map(|s| &s[..]) == Some(&self.label))
        {
            orders.push(CompletionOrder::NameMatched);
        }
        #[allow(clippy::blocks_in_if_conditions)]
        if self
            .arg_pt
            .map_or(false, |pt| self.mod_ctx.subtype_of(&self.vi.t, pt.typ()))
        {
            orders.push(CompletionOrder::TypeMatched);
        } else if self.arg_pt.map_or(false, |pt| {
            let Some(return_t) = self.vi.t.return_t() else { return false; };
            if return_t.has_qvar() {
                return false;
            }
            self.mod_ctx.subtype_of(return_t, pt.typ())
        }) {
            orders.push(CompletionOrder::ReturnTypeMatched);
        }
        orders.into_iter().map(i32::from).sum()
    }

    pub fn mangle(&self) -> String {
        let score = self.score();
        format!("{}_{}", char::from_u32(score as u32).unwrap(), self.label)
    }

    fn set(&self, item: &mut CompletionItem) {
        item.sort_text = Some(self.mangle());
    }
}

impl<Checker: BuildRunnable> Server<Checker> {
    pub(crate) fn show_completion(&mut self, msg: &Value) -> ELSResult<()> {
        send_log(format!("completion requested: {msg}"))?;
        let params = CompletionParams::deserialize(&msg["params"])?;
        let uri = util::normalize_url(params.text_document_position.text_document.uri);
        let path = util::uri_to_path(&uri);
        let pos = params.text_document_position.position;
        // ignore comments
        // TODO: multiline comments
        if self
            .file_cache
            .get(&uri)
            .unwrap()
            .get_line(pos.line)
            .map_or(false, |line| line.starts_with('#'))
        {
            return Ok(());
        }
        let trigger = params
            .context
            .as_ref()
            .and_then(|comp_ctx| comp_ctx.trigger_character.as_ref().map(|s| &s[..]));
        let comp_kind = match trigger {
            Some(".") => CompletionKind::Method,
            Some(":") => CompletionKind::Method,
            Some(" ") => CompletionKind::Space,
            Some("(") => CompletionKind::LParen,
            _ => CompletionKind::Local,
        };
        send_log(format!("CompletionKind: {comp_kind:?}"))?;
        let mut result: Vec<CompletionItem> = vec![];
        let mut already_appeared = Set::new();
        let contexts = if comp_kind.should_be_local() {
            let prev_token = self.file_cache.get_token_relatively(&uri, pos, -1);
            if prev_token
                .as_ref()
                .map(|t| matches!(t.kind, Dot | DblColon))
                .unwrap_or(false)
            {
                let dot_pos = util::loc_to_pos(prev_token.unwrap().loc()).unwrap();
                self.get_receiver_ctxs(&uri, dot_pos)?
            } else {
                self.get_local_ctx(&uri, pos)
            }
        } else {
            self.get_receiver_ctxs(&uri, pos)?
        };
        let offset = match comp_kind {
            CompletionKind::Local => 0,
            CompletionKind::Method => -1,
            CompletionKind::Space => -1,
            CompletionKind::LParen => 0,
        };
        let arg_pt = self
            .get_min_expr(&uri, pos, offset)
            .and_then(|(token, expr)| match expr {
                Expr::Call(call) => {
                    let sig_t = call.obj.t();
                    let nth = self.nth(&uri, call.args.loc(), &token);
                    let additional = if matches!(token.kind, Comma) { 1 } else { 0 };
                    let nth = nth + additional;
                    sig_t.non_default_params()?.get(nth).cloned()
                }
                other if comp_kind == CompletionKind::Space => {
                    let sig_t = other.t();
                    sig_t.non_default_params()?.get(0).cloned()
                }
                _ => None,
            });
        let mod_ctx = &self.modules.get(&uri).unwrap().context;
        for (name, vi) in contexts.into_iter().flat_map(|ctx| ctx.dir()) {
            if comp_kind.should_be_method() && vi.vis.is_private() {
                continue;
            }
            let label = name.to_string();
            // don't show overriden items
            if already_appeared.contains(&label) {
                continue;
            }
            // don't show future defined items
            if vi.def_loc.module.as_ref() == Some(&path)
                && name.ln_begin().unwrap_or(0) > pos.line + 1
            {
                continue;
            }
            let readable_t = self
                .modules
                .get(&uri)
                .map(|module| {
                    module
                        .context
                        .readable_type(vi.t.clone(), vi.kind.is_parameter())
                })
                .unwrap_or_else(|| vi.t.clone());
            let mut item = CompletionItem::new_simple(label, readable_t.to_string());
            CompletionOrderSetter::new(vi, arg_pt.as_ref(), mod_ctx, item.label.clone())
                .set(&mut item);
            item.kind = match &vi.t {
                Type::Subr(subr) if subr.self_t().is_some() => Some(CompletionItemKind::METHOD),
                Type::Quantified(quant) if quant.self_t().is_some() => {
                    Some(CompletionItemKind::METHOD)
                }
                Type::Subr(_) | Type::Quantified(_) => Some(CompletionItemKind::FUNCTION),
                Type::ClassType => Some(CompletionItemKind::CLASS),
                Type::TraitType => Some(CompletionItemKind::INTERFACE),
                t if &t.qual_name()[..] == "Module" || &t.qual_name()[..] == "GenericModule" => {
                    Some(CompletionItemKind::MODULE)
                }
                _ if vi.muty.is_const() => Some(CompletionItemKind::CONSTANT),
                _ => Some(CompletionItemKind::VARIABLE),
            };
            item.data = Some(Value::String(vi.def_loc.to_string()));
            already_appeared.insert(item.label.clone());
            result.push(item);
        }
        if comp_kind.should_be_local() {
            self.show_module_completion(&mut result, &already_appeared);
        }
        send_log(format!("completion items: {}", result.len()))?;
        send(&json!({ "jsonrpc": "2.0", "id": msg["id"].as_i64().unwrap(), "result": result }))
    }

    fn show_module_completion(
        &self,
        comps: &mut Vec<CompletionItem>,
        already_appeared: &Set<String>,
    ) {
        for mod_name in BUILTIN_PYTHON_MODS {
            if already_appeared.contains(mod_name) {
                continue;
            }
            let mut item = CompletionItem::new_simple(
                format!("{mod_name} (import from std)"),
                "PyModule".to_string(),
            );
            item.sort_text = Some(format!("{}_{}", CompletionOrder::BUILTIN_MOD, item.label));
            item.kind = Some(CompletionItemKind::MODULE);
            let import = if cfg!(feature = "py_compatible") {
                format!("import {mod_name}\n")
            } else {
                format!("{mod_name} = pyimport \"{mod_name}\"\n")
            };
            item.additional_text_edits = Some(vec![TextEdit {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                new_text: import,
            }]);
            item.insert_text = Some(mod_name.to_string());
            comps.push(item);
        }
        #[cfg(not(feature = "py_compatible"))]
        for mod_name in BUILTIN_ERG_MODS {
            if already_appeared.contains(mod_name) {
                continue;
            }
            let mut item = CompletionItem::new_simple(
                format!("{mod_name} (import from std)"),
                "Module".to_string(),
            );
            item.sort_text = Some(format!("{}_{}", CompletionOrder::BUILTIN_MOD, item.label));
            item.kind = Some(CompletionItemKind::MODULE);
            let import = format!("{mod_name} = import \"{mod_name}\"\n");
            item.additional_text_edits = Some(vec![TextEdit {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                new_text: import,
            }]);
            item.insert_text = Some(mod_name.to_string());
            comps.push(item);
        }
    }

    pub(crate) fn resolve_completion(&self, msg: &Value) -> ELSResult<()> {
        send_log(format!("completion resolve requested: {msg}"))?;
        let mut item = CompletionItem::deserialize(&msg["params"])?;
        if let Some(data) = &item.data {
            let mut contents = vec![];
            let Ok(def_loc) = data.as_str().unwrap().parse::<AbsLocation>() else {
                return send(&json!({ "jsonrpc": "2.0", "id": msg["id"].as_i64().unwrap(), "result": item }));
            };
            self.show_doc_comment(None, &mut contents, &def_loc)?;
            let mut contents = contents.into_iter().map(mark_to_string).collect::<Vec<_>>();
            contents.sort_by_key(|cont| markdown_order(cont));
            item.documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: contents.join("\n"),
            }));
        }
        send(&json!({ "jsonrpc": "2.0", "id": msg["id"].as_i64().unwrap(), "result": item }))
    }
}
