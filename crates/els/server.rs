use std::cell::RefCell;
use std::io;
use std::io::{stdin, stdout, BufRead, Read, StdinLock, StdoutLock, Write};
use std::path::PathBuf;
use std::str::FromStr;

use erg_common::env::erg_path;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;

use erg_common::config::ErgConfig;
use erg_common::dict::Dict;
use erg_common::normalize_path;

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::build_hir::HIRBuilder;
use erg_compiler::context::{Context, ModuleContext};
use erg_compiler::erg_parser::token::TokenKind;
use erg_compiler::hir::HIR;
use erg_compiler::module::{SharedCompilerResource, SharedModuleIndex};

use lsp_types::{
    ClientCapabilities, CodeActionKind, CodeActionOptions, CodeActionProviderCapability,
    CompletionOptions, ExecuteCommandOptions, HoverProviderCapability, InitializeResult, OneOf,
    Position, SemanticTokenType, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensServerCapabilities, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url, WorkDoneProgressOptions,
};

use crate::hir_visitor::HIRVisitor;
use crate::message::{ErrorMessage, LogMessage};
use crate::util;

pub type ELSResult<T> = Result<T, Box<dyn std::error::Error>>;

pub type ErgLanguageServer = Server<HIRBuilder>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ELSFeatures {
    CodeAction,
    Completion,
    Diagnostic,
    FindReferences,
    GotoDefinition,
    Hover,
    InlayHint,
    Rename,
    SemanticTokens,
}

impl From<&str> for ELSFeatures {
    fn from(s: &str) -> Self {
        match s {
            "codeaction" | "codeAction" | "code-action" => ELSFeatures::CodeAction,
            "completion" => ELSFeatures::Completion,
            "diagnostic" => ELSFeatures::Diagnostic,
            "hover" => ELSFeatures::Hover,
            "semantictoken" | "semantictokens" | "semanticToken" | "semanticTokens"
            | "semantic-tokens" => ELSFeatures::SemanticTokens,
            "rename" => ELSFeatures::Rename,
            "inlayhint" | "inlayhints" | "inlayHint" | "inlayHints" | "inlay-hint"
            | "inlay-hints" => ELSFeatures::InlayHint,
            "findreferences" | "findReferences" | "find-references" => ELSFeatures::FindReferences,
            "gotodefinition" | "gotoDefinition" | "goto-completion" => ELSFeatures::GotoDefinition,
            _ => panic!("unknown feature: {s}"),
        }
    }
}

macro_rules! _log {
    ($($arg:tt)*) => {
        Self::send_log(format!($($arg)*)).unwrap();
    };
}

thread_local! {
    static INPUT: RefCell<StdinLock<'static>> = RefCell::new(stdin().lock());
    static OUTPUT: RefCell<StdoutLock<'static>> = RefCell::new(stdout().lock());
}

fn send_stdout<T: ?Sized + Serialize>(message: &T) -> ELSResult<()> {
    let msg = serde_json::to_string(message)?;
    OUTPUT.with(|out| {
        write!(
            out.borrow_mut(),
            "Content-Length: {}\r\n\r\n{}",
            msg.len(),
            msg
        )?;
        out.borrow_mut().flush()?;
        Ok(())
    })
}

fn read_line() -> io::Result<String> {
    let mut line = String::new();
    INPUT.with(|input| {
        input.borrow_mut().read_line(&mut line)?;
        Ok(line)
    })
}

fn read_exact(len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; len];
    INPUT.with(|input| {
        input.borrow_mut().read_exact(&mut buf)?;
        Ok(buf)
    })
}

/// A Language Server, which can be used any object implementing `BuildRunnable` internally by passing it as a generic parameter.
#[derive(Debug)]
pub struct Server<Checker: BuildRunnable = HIRBuilder> {
    pub(crate) cfg: ErgConfig,
    pub(crate) home: PathBuf,
    pub(crate) erg_path: PathBuf,
    pub(crate) client_capas: ClientCapabilities,
    pub(crate) modules: Dict<Url, ModuleContext>,
    pub(crate) hirs: Dict<Url, Option<HIR>>,
    _checker: std::marker::PhantomData<Checker>,
}

impl<Checker: BuildRunnable> Server<Checker> {
    pub fn new(cfg: ErgConfig) -> Self {
        Self {
            cfg,
            home: normalize_path(std::env::current_dir().unwrap()),
            erg_path: erg_path(), // already normalized
            client_capas: ClientCapabilities::default(),
            modules: Dict::new(),
            hirs: Dict::new(),
            _checker: std::marker::PhantomData,
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let msg = Value::from_str(&self.read_message()?)?;
            self.dispatch(msg)?;
        }
        // Ok(())
    }

    #[allow(clippy::field_reassign_with_default)]
    fn init(&mut self, msg: &Value, id: i64) -> ELSResult<()> {
        Self::send_log("initializing ELS")?;
        // #[allow(clippy::collapsible_if)]
        if msg.get("params").is_some() && msg["params"].get("capabilities").is_some() {
            self.client_capas = ClientCapabilities::deserialize(&msg["params"]["capabilities"])?;
            // Self::send_log(format!("set client capabilities: {:?}", self.client_capas))?;
        }
        let mut args = self.cfg.runtime_args.iter();
        let mut disabled_features = vec![];
        while let Some(&arg) = args.next() {
            if arg == "--disable" {
                if let Some(&feature) = args.next() {
                    disabled_features.push(ELSFeatures::from(feature));
                }
            }
        }
        let mut result = InitializeResult::default();
        result.capabilities = ServerCapabilities::default();
        result.capabilities.text_document_sync =
            Some(TextDocumentSyncCapability::from(TextDocumentSyncKind::FULL));
        let mut comp_options = CompletionOptions::default();
        comp_options.trigger_characters =
            Some(vec![".".to_string(), ":".to_string(), "(".to_string()]);
        result.capabilities.completion_provider = Some(comp_options);
        result.capabilities.rename_provider = Some(OneOf::Left(true));
        result.capabilities.references_provider = Some(OneOf::Left(true));
        result.capabilities.definition_provider = Some(OneOf::Left(true));
        result.capabilities.hover_provider = if disabled_features.contains(&ELSFeatures::Hover) {
            None
        } else {
            Some(HoverProviderCapability::Simple(true))
        };
        result.capabilities.inlay_hint_provider =
            if disabled_features.contains(&ELSFeatures::InlayHint) {
                None
            } else {
                Some(OneOf::Left(true))
            };
        let mut sema_options = SemanticTokensOptions::default();
        sema_options.range = Some(false);
        sema_options.full = Some(SemanticTokensFullOptions::Bool(true));
        sema_options.legend = SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::NAMESPACE,
                SemanticTokenType::TYPE,
                SemanticTokenType::CLASS,
                SemanticTokenType::INTERFACE,
                SemanticTokenType::TYPE_PARAMETER,
                SemanticTokenType::PARAMETER,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::PROPERTY,
                SemanticTokenType::FUNCTION,
                SemanticTokenType::METHOD,
                SemanticTokenType::STRING,
                SemanticTokenType::NUMBER,
                SemanticTokenType::OPERATOR,
            ],
            token_modifiers: vec![],
        };
        result.capabilities.semantic_tokens_provider =
            if disabled_features.contains(&ELSFeatures::SemanticTokens) {
                None
            } else {
                Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                    sema_options,
                ))
            };
        result.capabilities.code_action_provider = if disabled_features
            .contains(&ELSFeatures::CodeAction)
        {
            None
        } else {
            let options = CodeActionProviderCapability::Options(CodeActionOptions {
                code_action_kinds: Some(vec![CodeActionKind::QUICKFIX, CodeActionKind::REFACTOR]),
                resolve_provider: Some(false),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            });
            Some(options)
        };
        result.capabilities.execute_command_provider = Some(ExecuteCommandOptions {
            commands: vec!["els.eliminateUnusedVars".to_string()],
            work_done_progress_options: WorkDoneProgressOptions::default(),
        });
        Self::send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }

    fn exit(&self) -> ELSResult<()> {
        Self::send_log("exiting ELS")?;
        std::process::exit(0);
    }

    fn shutdown(&self, id: i64) -> ELSResult<()> {
        Self::send_log("shutting down ELS")?;
        Self::send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": json!(null),
        }))
    }

    /// Copied and modified from RLS, https://github.com/rust-lang/rls/blob/master/rls/src/server/io.rs
    fn read_message(&self) -> Result<String, io::Error> {
        // Read in the "Content-Length: xx" part.
        let mut size: Option<usize> = None;
        loop {
            let buffer = read_line()?;

            // End of input.
            if buffer.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF encountered in the middle of reading LSP headers",
                ));
            }

            // Header section is finished, break from the loop.
            if buffer == "\r\n" {
                break;
            }

            let res: Vec<&str> = buffer.split(' ').collect();

            // Make sure header is valid.
            if res.len() != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Header '{buffer}' is malformed"),
                ));
            }
            let header_name = res[0].to_lowercase();
            let header_value = res[1].trim();

            match header_name.as_ref() {
                "content-length:" => {
                    size = Some(header_value.parse::<usize>().map_err(|_e| {
                        io::Error::new(io::ErrorKind::InvalidData, "Couldn't read size")
                    })?);
                }
                "content-type:" => {
                    if header_value != "utf8" && header_value != "utf-8" {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Content type '{header_value}' is invalid"),
                        ));
                    }
                }
                // Ignore unknown headers (specification doesn't say what to do in this case).
                _ => (),
            }
        }
        let size = match size {
            Some(size) => size,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Message is missing 'content-length' header",
                ));
            }
        };

        let content = read_exact(size)?;

        String::from_utf8(content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn dispatch(&mut self, msg: Value) -> ELSResult<()> {
        match (
            msg.get("id").and_then(|i| i.as_i64()),
            msg.get("method").and_then(|m| m.as_str()),
        ) {
            (Some(id), Some(method)) => self.handle_request(&msg, id, method),
            (Some(_id), None) => {
                // ignore at this time
                Ok(())
            }
            (None, Some(notification)) => self.handle_notification(&msg, notification),
            _ => Self::send_invalid_req_error(),
        }
    }

    fn handle_request(&mut self, msg: &Value, id: i64, method: &str) -> ELSResult<()> {
        match method {
            "initialize" => self.init(msg, id),
            "shutdown" => self.shutdown(id),
            "textDocument/completion" => self.show_completion(msg),
            "textDocument/definition" => self.show_definition(msg),
            "textDocument/hover" => self.show_hover(msg),
            "textDocument/rename" => self.rename(msg),
            "textDocument/references" => self.show_references(msg),
            "textDocument/semanticTokens/full" => self.get_semantic_tokens_full(msg),
            "textDocument/inlayHint" => self.get_inlay_hint(msg),
            "textDocument/codeAction" => self.perform_code_action(msg),
            "workspace/executeCommand" => self.execute_command(msg),
            other => Self::send_error(Some(id), -32600, format!("{other} is not supported")),
        }
    }

    fn handle_notification(&mut self, msg: &Value, method: &str) -> ELSResult<()> {
        match method {
            "initialized" => Self::send_log("successfully bound"),
            "exit" => self.exit(),
            "textDocument/didOpen" => {
                let uri = util::parse_and_normalize_url(
                    msg["params"]["textDocument"]["uri"].as_str().unwrap(),
                )?;
                Self::send_log(format!("{method}: {uri}"))?;
                self.check_file(uri, msg["params"]["textDocument"]["text"].as_str().unwrap())
            }
            "textDocument/didSave" => {
                let uri = util::parse_and_normalize_url(
                    msg["params"]["textDocument"]["uri"].as_str().unwrap(),
                )?;
                Self::send_log(format!("{method}: {uri}"))?;
                let code = util::get_code_from_uri(&uri)?;
                self.clear_cache(&uri);
                self.check_file(uri, &code)
            }
            // "textDocument/didChange"
            _ => Self::send_log(format!("received notification: {method}")),
        }
    }

    pub(crate) fn send<T: ?Sized + Serialize>(message: &T) -> ELSResult<()> {
        send_stdout(message)
    }

    pub(crate) fn send_log<S: Into<String>>(msg: S) -> ELSResult<()> {
        Self::send(&LogMessage::new(msg))
    }

    pub(crate) fn send_error<S: Into<String>>(id: Option<i64>, code: i64, msg: S) -> ELSResult<()> {
        Self::send(&ErrorMessage::new(
            id,
            json!({ "code": code, "message": msg.into() }),
        ))
    }

    pub(crate) fn send_invalid_req_error() -> ELSResult<()> {
        Self::send_error(None, -32601, "received an invalid request")
    }

    pub(crate) fn get_visitor(&self, uri: &Url) -> Option<HIRVisitor> {
        self.hirs
            .get(uri)?
            .as_ref()
            .map(|hir| HIRVisitor::new(hir, uri.clone(), !cfg!(feature = "py_compatible")))
    }

    pub(crate) fn get_local_ctx(&self, uri: &Url, pos: Position) -> Vec<&Context> {
        // Self::send_log(format!("scope: {:?}\n", self.module.as_ref().unwrap().scope.keys())).unwrap();
        let mut ctxs = vec![];
        if let Some(visitor) = self.get_visitor(uri) {
            let ns = visitor.get_namespace(pos);
            Self::send_log(format!("ns: {ns:?}")).unwrap();
            for i in 1..ns.len() {
                let ns = ns[..=ns.len() - i].join("");
                if let Some(ctx) = self.modules.get(uri).unwrap().scope.get(&ns[..]) {
                    ctxs.push(ctx);
                }
            }
        }
        ctxs.push(&self.modules.get(uri).unwrap().context);
        ctxs
    }

    pub(crate) fn get_receiver_ctxs(
        &self,
        uri: &Url,
        attr_marker_pos: Position,
    ) -> ELSResult<Vec<&Context>> {
        let Some(module) = self.modules.get(uri) else {
            return Ok(vec![]);
        };
        let maybe_token = util::get_token_relatively(uri.clone(), attr_marker_pos, -1)?;
        if let Some(token) = maybe_token {
            if token.is(TokenKind::Symbol) {
                let var_name = token.inspect();
                Self::send_log(format!("{} name: {var_name}", line!()))?;
                Ok(module.context.get_receiver_ctxs(var_name))
            } else {
                Self::send_log(format!("non-name token: {token}"))?;
                if let Some(typ) = self
                    .get_visitor(uri)
                    .and_then(|visitor| visitor.get_t(&token))
                {
                    let t_name = typ.qual_name();
                    Self::send_log(format!("type: {t_name}"))?;
                    Ok(module.context.get_receiver_ctxs(&t_name))
                } else {
                    Ok(vec![])
                }
            }
        } else {
            Self::send_log("token not found")?;
            Ok(vec![])
        }
    }

    pub(crate) fn get_index(&self) -> &SharedModuleIndex {
        self.modules
            .values()
            .next()
            .unwrap()
            .context
            .index()
            .unwrap()
    }

    pub(crate) fn get_shared(&self) -> Option<&SharedCompilerResource> {
        self.modules
            .values()
            .next()
            .and_then(|module| module.context.shared())
    }

    pub(crate) fn clear_cache(&mut self, uri: &Url) {
        self.hirs.remove(uri);
        if let Some(module) = self.modules.remove(uri) {
            if let Some(shared) = module.context.shared() {
                shared.mod_cache.remove(&util::uri_to_path(uri));
            }
        }
    }
}
