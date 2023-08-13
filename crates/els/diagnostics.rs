use std::env::current_dir;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use erg_common::consts::PYTHON_MODE;
use erg_common::dict::Dict;
use erg_common::fn_name;
use erg_common::spawn::spawn_new_thread;
use erg_common::style::*;
use erg_common::traits::Stream;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::ast::Module;
use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::error::CompileErrors;

use lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, Position, ProgressParams, ProgressParamsValue,
    PublishDiagnosticsParams, Range, Url, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkDoneProgressReport,
};
use serde_json::json;

use crate::_log;
use crate::diff::{ASTDiff, HIRDiff};
use crate::server::{send, send_log, AnalysisResult, DefaultFeatures, ELSResult, Server};
use crate::util::{self, project_root_of, NormalizedUrl};

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn get_ast(&self, uri: &NormalizedUrl) -> Option<Module> {
        let code = self.file_cache.get_entire_code(uri).ok()?;
        Parser::parse(code).ok().map(|artifact| artifact.ast)
    }

    pub(crate) fn check_file<S: Into<String>>(
        &mut self,
        uri: NormalizedUrl,
        code: S,
    ) -> ELSResult<()> {
        send_log(format!("checking {uri}"))?;
        let path = util::uri_to_path(&uri);
        let mode = if path.to_string_lossy().ends_with(".d.er") {
            "declare"
        } else {
            "exec"
        };
        let mut checker = self.get_checker(path.clone());
        let artifact = match checker.build(code.into(), mode) {
            Ok(artifact) => {
                send_log(format!(
                    "checking {uri} passed, found warns: {}",
                    artifact.warns.len()
                ))?;
                let uri_and_diags = self.make_uri_and_diags(artifact.warns.clone());
                // clear previous diagnostics
                self.send_diagnostics(uri.clone().raw(), vec![])?;
                for (uri, diags) in uri_and_diags.into_iter() {
                    send_log(format!("{uri}, warns: {}", diags.len()))?;
                    self.send_diagnostics(uri, diags)?;
                }
                artifact.into()
            }
            Err(artifact) => {
                send_log(format!("found errors: {}", artifact.errors.len()))?;
                send_log(format!("found warns: {}", artifact.warns.len()))?;
                let diags = artifact
                    .errors
                    .clone()
                    .into_iter()
                    .chain(artifact.warns.clone().into_iter())
                    .collect();
                let uri_and_diags = self.make_uri_and_diags(diags);
                if uri_and_diags.is_empty() {
                    self.send_diagnostics(uri.clone().raw(), vec![])?;
                }
                for (uri, diags) in uri_and_diags.into_iter() {
                    send_log(format!("{uri}, errs & warns: {}", diags.len()))?;
                    self.send_diagnostics(uri, diags)?;
                }
                artifact
            }
        };
        if let Some(shared) = self.get_shared() {
            if mode == "declare" {
                shared.py_mod_cache.register(
                    path,
                    artifact.object.clone(),
                    checker.get_context().unwrap().clone(),
                );
            } else {
                shared.mod_cache.register(
                    path,
                    artifact.object.clone(),
                    checker.get_context().unwrap().clone(),
                );
            }
        }
        if let Some(module) = self.get_ast(&uri) {
            self.analysis_result
                .insert(uri.clone(), AnalysisResult::new(module, artifact));
        }
        if let Some(module) = checker.pop_context() {
            send_log(format!("{uri}: {}", module.context.name))?;
            self.modules.insert(uri.clone(), module);
        }
        let dependents = self.dependents_of(&uri);
        for dep in dependents {
            // _log!("dep: {dep}");
            let code = self.file_cache.get_entire_code(&dep)?.to_string();
            self.check_file(dep, code)?;
        }
        Ok(())
    }

    pub(crate) fn quick_check_file(&mut self, uri: NormalizedUrl) -> ELSResult<()> {
        let Some(old) = self.analysis_result.get_ast(&uri) else {
            crate::_log!("not found");
            return Ok(());
        };
        let Some(new) = self.get_ast(&uri) else {
            crate::_log!("not found");
            return Ok(());
        };
        let ast_diff = ASTDiff::diff(old, &new);
        crate::_log!("diff: {ast_diff}");
        if let Some(mut lowerer) = self.steal_lowerer(&uri) {
            let hir = self.analysis_result.get_mut_hir(&uri);
            if let Some((hir_diff, hir)) = HIRDiff::new(ast_diff, &mut lowerer).zip(hir) {
                crate::_log!("hir_diff: {hir_diff}");
                hir_diff.update(hir);
            }
            self.restore_lowerer(uri, lowerer);
        }
        // skip checking for dependents
        Ok(())
    }

    fn make_uri_and_diags(&mut self, errors: CompileErrors) -> Vec<(Url, Vec<Diagnostic>)> {
        let mut uri_and_diags: Vec<(Url, Vec<Diagnostic>)> = vec![];
        for err in errors.into_iter() {
            let loc = err.core.get_loc_with_fallback();
            let res_uri = Url::from_file_path(
                err.input
                    .path()
                    .canonicalize()
                    .unwrap_or(err.input.path().to_path_buf()),
            );
            let Ok(err_uri) = res_uri else {
                crate::_log!("failed to get uri: {}", err.input.path().display());
                continue;
            };
            let mut message = remove_style(&err.core.main_message);
            for sub in err.core.sub_messages {
                for msg in sub.get_msg() {
                    message.push('\n');
                    message.push_str(&remove_style(msg));
                }
                if let Some(hint) = sub.get_hint() {
                    message.push('\n');
                    message.push_str("hint: ");
                    message.push_str(&remove_style(hint));
                }
            }
            let start = Position::new(
                loc.ln_begin().unwrap_or(1) - 1,
                loc.col_begin().unwrap_or(0),
            );
            let end = Position::new(loc.ln_end().unwrap_or(1) - 1, loc.col_end().unwrap_or(0));
            let severity = if err.core.kind.is_warning() {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            };
            let source = if PYTHON_MODE { "pylyzer" } else { "els" };
            let diag = Diagnostic::new(
                Range::new(start, end),
                Some(severity),
                Some(NumberOrString::String(format!("E{}", err.core.errno))),
                Some(source.to_string()),
                message,
                None,
                None,
            );
            if let Some((_, diags)) = uri_and_diags.iter_mut().find(|x| x.0 == err_uri) {
                diags.push(diag);
            } else {
                uri_and_diags.push((err_uri, vec![diag]));
            }
        }
        uri_and_diags
    }

    fn send_diagnostics(&self, uri: Url, diagnostics: Vec<Diagnostic>) -> ELSResult<()> {
        if self
            .disabled_features
            .contains(&DefaultFeatures::Diagnostics)
        {
            return Ok(());
        }
        let params = PublishDiagnosticsParams::new(uri, diagnostics, None);
        if self
            .client_capas
            .text_document
            .as_ref()
            .map(|doc| doc.publish_diagnostics.is_some())
            .unwrap_or(false)
        {
            send(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": params,
            }))?;
        } else {
            send_log("the client does not support diagnostics")?;
        }
        Ok(())
    }

    /// Periodically send diagnostics without a request from the server.
    /// This is necessary to perform reactive error highlighting in editors such as Vim, where no action is taken until the buffer is saved.
    pub(crate) fn start_auto_diagnostics(&mut self) {
        let mut _self = self.clone();
        spawn_new_thread(
            move || {
                let mut file_vers = Dict::<NormalizedUrl, i32>::new();
                loop {
                    for uri in _self.file_cache.entries() {
                        let Some(latest_ver) = _self.file_cache.get_ver(&uri) else {
                            continue;
                        };
                        let Some(&ver) = file_vers.get(&uri) else {
                            file_vers.insert(uri.clone(), latest_ver);
                            continue;
                        };
                        if latest_ver != ver {
                            if let Ok(code) = _self.file_cache.get_entire_code(&uri) {
                                let _ = _self.check_file(uri.clone(), code);
                                file_vers.insert(uri, latest_ver);
                            }
                        }
                    }
                    sleep(Duration::from_millis(500));
                }
            },
            fn_name!(),
        );
    }

    fn project_files(dir: PathBuf) -> Vec<NormalizedUrl> {
        let mut uris = vec![];
        for entry in dir.read_dir().unwrap() {
            let Ok(entry) = entry else {
                continue;
            };
            if entry.path().extension() == Some(OsStr::new("er")) {
                let uri = NormalizedUrl::from_file_path(entry.path()).unwrap();
                uris.push(uri);
            } else if entry.path().is_dir() {
                uris.extend(Self::project_files(entry.path()));
            }
        }
        uris
    }

    pub(crate) fn start_workspace_diagnostics(&mut self) {
        let mut _self = self.clone();
        spawn_new_thread(
            move || {
                _log!("start workspace diagnostics");
                sleep(Duration::from_secs(1));
                let token = NumberOrString::String("els/start_workspace_diagnostics".to_string());
                let progress_token = WorkDoneProgressCreateParams {
                    token: token.clone(),
                };
                let _ = send(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "window/workDoneProgress/create",
                    "params": progress_token,
                }));
                let project_root = project_root_of(&current_dir().unwrap());
                let src_dir = if project_root.join("src").exists() {
                    project_root.join("src")
                } else {
                    project_root
                };
                let uris = Self::project_files(src_dir);
                let progress_begin = WorkDoneProgressBegin {
                    title: "Checking workspace".to_string(),
                    cancellable: Some(false),
                    message: Some(format!("checked 0/{} files", uris.len())),
                    percentage: Some(0),
                };
                let params = ProgressParams {
                    token: token.clone(),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(progress_begin)),
                };
                let _ = send(&json!({
                    "jsonrpc": "2.0",
                    "method": "$/progress",
                    "params": params,
                }));
                let len = uris.len();
                for (i, uri) in uris.into_iter().enumerate() {
                    let code = _self.file_cache.get_entire_code(&uri).unwrap();
                    let _ = _self.check_file(uri, code);
                    let percentage = (i + 1) as f64 / len as f64 * 100.0;
                    let progress = WorkDoneProgressReport {
                        cancellable: Some(false),
                        message: Some(format!("checked {}/{len} files", i + 1)),
                        percentage: Some(percentage as u32),
                    };
                    let params = ProgressParams {
                        token: token.clone(),
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(progress)),
                    };
                    let _ = send(&json!({
                        "jsonrpc": "2.0",
                        "method": "$/progress",
                        "params": params,
                    }));
                }
                let progress_end = WorkDoneProgressEnd {
                    message: Some(format!("checked {len} files")),
                };
                let params = ProgressParams {
                    token: token.clone(),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(progress_end)),
                };
                let _ = send(&json!({
                    "jsonrpc": "2.0",
                    "method": "$/progress",
                    "params": params,
                }));
            },
            fn_name!(),
        );
    }
}
