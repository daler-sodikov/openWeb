//! Code insight commands backed by the language server: hover, navigation,
//! diagnostics, problems and server status (`docs/ARCHITECTURE.md`, 4.7).
//!
//! Each request locks the document briefly (sync, position, send), then
//! waits for the server without any lock held.

use std::path::PathBuf;
use std::sync::Arc;

use ferrule_buffer::{LinePos, Rope};
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_lsp_client::lsp_types::request::{
    GotoDefinition, GotoImplementation, GotoTypeDefinition, HoverRequest, Request,
};
use ferrule_lsp_client::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, HoverContents, HoverParams, MarkedString,
    MarkupKind, PartialResultParams, Position, Range, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkDoneProgressParams,
};
use ferrule_lsp_client::{PendingRequest, Supervisor, uri_to_path};

use crate::convert::{buffer_error, to_u32, to_usize};
use crate::hover_html::{plain_text_html, render_markdown};
use crate::lsp::{LanguageService, diagnostic_spans, lsp_error};
use crate::session::{DocHandle, DocState, Session, blocking};

fn check_version(state: &DocState, version: u32) -> Result<(), IpcError> {
    if state.doc.version() == version {
        Ok(())
    } else {
        Err(buffer_error(ferrule_buffer::BufferError::VersionMismatch {
            expected: version,
            actual: state.doc.version(),
        }))
    }
}

fn position(state: &DocState, offset: u32) -> Position {
    let pos = state.doc.line_pos(to_usize(offset));
    Position::new(to_u32(pos.line), to_u32(pos.character))
}

fn text_position(uri: &Uri, position: Position) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position,
    }
}

/// Markdown of a hover, or `None` for plain text.
fn hover_markdown(contents: HoverContents) -> Result<String, String> {
    let marked = |m: MarkedString| match m {
        MarkedString::String(text) => text,
        MarkedString::LanguageString(code) => {
            format!("```{}\n{}\n```", code.language, code.value)
        }
    };
    match contents {
        HoverContents::Scalar(m) => Ok(marked(m)),
        HoverContents::Array(items) => Ok(items
            .into_iter()
            .map(marked)
            .collect::<Vec<_>>()
            .join("\n\n")),
        HoverContents::Markup(markup) => match markup.kind {
            MarkupKind::Markdown => Ok(markup.value),
            MarkupKind::PlainText => Err(markup.value),
        },
    }
}

fn targets(response: Option<GotoDefinitionResponse>) -> Vec<(Uri, Range)> {
    match response {
        None => Vec::new(),
        Some(GotoDefinitionResponse::Scalar(location)) => vec![(location.uri, location.range)],
        Some(GotoDefinitionResponse::Array(locations)) => locations
            .into_iter()
            .map(|location| (location.uri, location.range))
            .collect(),
        Some(GotoDefinitionResponse::Link(links)) => links
            .into_iter()
            .map(|link| (link.target_uri, link.target_selection_range))
            .collect(),
    }
}

fn start_navigation<R>(
    supervisor: &Supervisor,
    params: GotoDefinitionParams,
) -> Result<PendingRequest<Option<GotoDefinitionResponse>>, IpcError>
where
    R: Request<Params = GotoDefinitionParams, Result = Option<GotoDefinitionResponse>>,
{
    supervisor.start_request::<R>(params).map_err(lsp_error)
}

impl Session {
    /// The open document and the project's language service.
    fn insight(&self, doc_id: DocId) -> Result<(Arc<DocHandle>, Arc<LanguageService>), IpcError> {
        let runtime = self.runtime()?;
        let handle = self.doc(doc_id)?;
        Ok((handle, runtime.lsp.clone()))
    }

    /// Hover / Quick Documentation at a UTF-16 offset of `version`. `None`
    /// when there is nothing to show or the server is not running; a newer
    /// hover for the same document cancels this one (`Cancelled`).
    pub async fn hover(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
    ) -> Result<Option<ipc::HoverResult>, IpcError> {
        let (handle, service) = self.insight(doc_id)?;
        let Some(uri) = handle.uri.clone().filter(|_| handle.lsp_language.is_some()) else {
            return Ok(None);
        };
        let token = service.hover_token(doc_id);
        let h = handle.clone();
        let pending = blocking(move || {
            let mut state = h.lock()?;
            check_version(&state, version)?;
            let Some(supervisor) = service.prepare_request(&h, &mut state) else {
                return Ok(None);
            };
            let params = HoverParams {
                text_document_position_params: text_position(&uri, position(&state, offset)),
                work_done_progress_params: WorkDoneProgressParams::default(),
            };
            supervisor
                .start_request::<HoverRequest>(params)
                .map(Some)
                .map_err(lsp_error)
        })
        .await?;
        let Some(pending) = pending else {
            return Ok(None);
        };
        let Some(hover) = pending.wait(&token).await.map_err(lsp_error)? else {
            return Ok(None);
        };
        let registry = self.registry.clone();
        blocking(move || {
            let html = match hover_markdown(hover.contents) {
                Ok(markdown) => render_markdown(&markdown, &registry),
                Err(plain) => plain_text_html(&plain),
            };
            if html.trim().is_empty() {
                return Ok(None);
            }
            let state = handle.lock()?;
            let doc = &state.doc;
            let offset_of = |p: Position| {
                to_u32(doc.offset_of(LinePos::new(p.line as usize, p.character as usize)))
            };
            let (from, to) = match hover.range {
                Some(range) => (offset_of(range.start), offset_of(range.end)),
                None => (offset, offset),
            };
            Ok(Some(ipc::HoverResult {
                version: doc.version(),
                from,
                to,
                html,
            }))
        })
        .await
    }

    /// Go to Declaration / Type Declaration / Implementation at a UTF-16
    /// offset of `version`. A newer navigation cancels this one.
    pub async fn goto_definition(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
        kind: ipc::NavKind,
    ) -> Result<Vec<ipc::NavTarget>, IpcError> {
        let (handle, service) = self.insight(doc_id)?;
        let Some(uri) = handle.uri.clone().filter(|_| handle.lsp_language.is_some()) else {
            return Ok(Vec::new());
        };
        let token = service.nav_token();
        let status = service.status();
        let h = handle.clone();
        let pending = blocking(move || {
            let mut state = h.lock()?;
            check_version(&state, version)?;
            let Some(supervisor) = service.prepare_request(&h, &mut state) else {
                return Err(IpcError::new(
                    IpcErrorKind::Conflict,
                    format!("{} is not ready ({})", status.name, status.message),
                ));
            };
            let params = GotoDefinitionParams {
                text_document_position_params: text_position(&uri, position(&state, offset)),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            };
            match kind {
                ipc::NavKind::Definition => start_navigation::<GotoDefinition>(&supervisor, params),
                ipc::NavKind::TypeDefinition => {
                    start_navigation::<GotoTypeDefinition>(&supervisor, params)
                }
                ipc::NavKind::Implementation => {
                    start_navigation::<GotoImplementation>(&supervisor, params)
                }
            }
        })
        .await?;
        let response = pending.wait(&token).await.map_err(lsp_error)?;
        let found = targets(response);
        let runtime = self.runtime()?;
        let open: Vec<(PathBuf, Arc<DocHandle>)> = self
            .open_docs()
            .into_iter()
            .map(|d| (d.path.clone(), d))
            .collect();
        blocking(move || {
            let mut result: Vec<ipc::NavTarget> = Vec::new();
            for (uri, range) in found {
                let Some(path) = uri_to_path(&uri) else {
                    continue;
                };
                let line = range.start.line;
                let preview = match open.iter().find(|(p, _)| *p == path) {
                    Some((_, doc)) => doc
                        .lock()
                        .ok()
                        .map(|state| line_text(state.doc.rope(), line))
                        .unwrap_or_default(),
                    None => ferrule_vfs::read_text(&path)
                        .map(|file| line_text(&Rope::from_str(&file.text), line))
                        .unwrap_or_default(),
                };
                let relative = runtime.project.relativize(&path);
                let target = ipc::NavTarget {
                    in_project: relative.is_some(),
                    path: relative.unwrap_or_else(|| path.to_string_lossy().into_owned()),
                    line,
                    column: range.start.character,
                    preview,
                };
                if !result.contains(&target) {
                    result.push(target);
                }
            }
            Ok(result)
        })
        .await
    }

    /// Diagnostics of an open document as offsets in its current text.
    pub async fn doc_diagnostics(&self, doc_id: DocId) -> Result<ipc::DocDiagnostics, IpcError> {
        let (handle, service) = self.insight(doc_id)?;
        blocking(move || {
            let diagnostics = service.diagnostics_for(&handle.rel_path);
            let state = handle.lock()?;
            Ok(ipc::DocDiagnostics {
                version: state.doc.version(),
                items: diagnostic_spans(&state.doc, &diagnostics),
            })
        })
        .await
    }

    /// Problems of every file, as last published by the server.
    pub async fn problems(&self) -> Result<Vec<ipc::FileProblems>, IpcError> {
        Ok(self.runtime()?.lsp.problems())
    }

    /// Status of the project's language servers.
    pub async fn language_servers(&self) -> Result<Vec<ipc::ServerStatus>, IpcError> {
        Ok(match self.runtime() {
            Ok(runtime) => vec![runtime.lsp.status()],
            Err(_) => Vec::new(),
        })
    }

    /// Restarts the TypeScript server (also after it gave up).
    pub async fn restart_language_server(&self) -> Result<(), IpcError> {
        let service = self.runtime()?.lsp.clone();
        service.restart().await;
        Ok(())
    }
}

/// One line of text, trimmed, for previews.
fn line_text(rope: &Rope, line: u32) -> String {
    let line = line as usize;
    if line >= rope.len_lines() {
        return String::new();
    }
    let text: String = rope.line(line).chars().take(300).collect();
    text.trim().to_owned()
}
