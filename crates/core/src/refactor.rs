//! Editing features from the language server (`docs/ARCHITECTURE.md`,
//! 4.8): completion, signature help, usages, rename, code actions,
//! organize/sort imports and formatting.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ferrule_buffer::{LinePos, Rope};
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_lsp_client::lsp_types::request::{
    CodeActionRequest, CodeActionResolveRequest, Completion, ExecuteCommand, Formatting,
    PrepareRenameRequest, References, Rename, ResolveCompletionItem, SignatureHelpRequest,
};
use ferrule_lsp_client::lsp_types::{
    self as lsp, CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionTriggerKind, CompletionContext, CompletionParams, CompletionResponse,
    CompletionTriggerKind, DocumentFormattingParams, Documentation, ExecuteCommandParams,
    FormattingOptions, MarkupKind, OneOf, ParameterLabel, PartialResultParams, Position,
    PrepareRenameResponse, ReferenceContext, ReferenceParams, RenameParams, SignatureHelpContext,
    SignatureHelpParams, SignatureHelpTriggerKind, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkDoneProgressParams, WorkspaceEdit,
};
use ferrule_lsp_client::{LspError, PendingRequest, Supervisor, uri_to_path};
use tokio_util::sync::CancellationToken;

use crate::completion::{CompletionSession, accept_edit, rank};
use crate::convert::{buffer_error, to_u32, to_usize};
use crate::edits::{self, Expect, FileEdits, is_library};
use crate::hover_html::{plain_text_html, render_markdown};
use crate::lsp::{LanguageService, Refactoring, lsp_error};
use crate::session::{DocHandle, DocState, Session, blocking};

/// Retries of `completionItem/resolve` the server cancelled (project loading).
const RESOLVE_RETRIES: u32 = 5;
const RESOLVE_RETRY_DELAY: Duration = Duration::from_millis(200);

/// Longest wait for a command the server runs for a code action.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

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

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// UTF-16 start of the identifier ending at `offset`.
fn word_start(rope: &Rope, offset: usize) -> usize {
    let mut char_idx = rope.utf16_cu_to_char(offset.min(rope.len_utf16_cu()));
    while char_idx > 0 && is_word(rope.char(char_idx - 1)) {
        char_idx -= 1;
    }
    rope.char_to_utf16_cu(char_idx)
}

/// The identifier around `offset` (UTF-16 range and text).
fn word_at(rope: &Rope, offset: usize) -> (usize, usize, String) {
    let start = word_start(rope, offset);
    let mut end_char = rope.utf16_cu_to_char(offset.min(rope.len_utf16_cu()));
    while end_char < rope.len_chars() && is_word(rope.char(end_char)) {
        end_char += 1;
    }
    let end = rope.char_to_utf16_cu(end_char);
    let text = rope
        .slice(rope.utf16_cu_to_char(start)..end_char)
        .to_string();
    (start, end, text)
}

fn documentation_html(
    doc: Option<&Documentation>,
    registry: &ferrule_syntax::LanguageRegistry,
) -> Option<String> {
    let html = match doc? {
        Documentation::String(text) => plain_text_html(text),
        Documentation::MarkupContent(markup) => match markup.kind {
            MarkupKind::Markdown => render_markdown(&markup.value, registry),
            MarkupKind::PlainText => plain_text_html(&markup.value),
        },
    };
    (!html.trim().is_empty()).then_some(html)
}

/// One line of text, trimmed, for previews.
pub(crate) fn line_text(rope: &Rope, line: u32) -> String {
    let line = line as usize;
    if line >= rope.len_lines() {
        return String::new();
    }
    let text: String = rope.line(line).chars().take(300).collect();
    text.trim().to_owned()
}

/// A request prepared under the document lock: the server has the current
/// text and the request is already queued after it.
struct Prepared<T> {
    pending: PendingRequest<T>,
    handle: Arc<DocHandle>,
    service: Arc<LanguageService>,
}

impl Session {
    /// Runs `build` with the document locked, synced and flushed to the
    /// server; `None` when the document has no server or it is not running.
    async fn prepare<T: Send + 'static>(
        &self,
        doc_id: DocId,
        version: Option<u32>,
        build: impl FnOnce(&Supervisor, &DocState, &Uri) -> Result<PendingRequest<T>, IpcError>
        + Send
        + 'static,
    ) -> Result<Option<Prepared<T>>, IpcError> {
        let runtime = self.runtime()?;
        let handle = self.doc(doc_id)?;
        let service = runtime.lsp.clone();
        let Some(uri) = handle.uri.clone().filter(|_| handle.lsp_language.is_some()) else {
            return Ok(None);
        };
        let h = handle.clone();
        let s = service.clone();
        let pending = blocking(move || {
            let mut state = h.lock()?;
            if let Some(version) = version {
                check_version(&state, version)?;
            }
            let Some(supervisor) = s.prepare_request(&h, &mut state) else {
                return Ok(None);
            };
            build(&supervisor, &state, &uri).map(Some)
        })
        .await?;
        Ok(pending.map(|pending| Prepared {
            pending,
            handle,
            service,
        }))
    }

    /// Like [`Session::prepare`], but a server that is not running is an
    /// error naming its state (for explicit actions such as Rename).
    async fn prepare_required<T: Send + 'static>(
        &self,
        doc_id: DocId,
        version: u32,
        build: impl FnOnce(&Supervisor, &DocState, &Uri) -> Result<PendingRequest<T>, IpcError>
        + Send
        + 'static,
    ) -> Result<Prepared<T>, IpcError> {
        let status = self.runtime()?.lsp.status();
        self.prepare(doc_id, Some(version), build)
            .await?
            .ok_or_else(|| {
                IpcError::new(
                    IpcErrorKind::Conflict,
                    format!("{} is not ready ({})", status.name, status.message),
                )
            })
    }

    // Completion

    /// Asks the server for completions at `offset`; `trigger` is the typed
    /// trigger character (`None`: invoked with Ctrl+Space or by typing a
    /// word). A newer request for the same document cancels this one.
    pub async fn completion(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
        trigger: Option<String>,
    ) -> Result<Option<ipc::CompletionList>, IpcError> {
        let token = self.runtime()?.lsp.token("completion", doc_id);
        let from_box = Arc::new(std::sync::Mutex::new((0_usize, String::new())));
        let from_out = from_box.clone();
        let Some(prepared) = self
            .prepare(doc_id, Some(version), move |supervisor, state, uri| {
                let rope = state.doc.rope();
                let at = to_usize(offset);
                let from = word_start(rope, at);
                let prefix = rope
                    .slice(
                        rope.utf16_cu_to_char(from)
                            ..rope.utf16_cu_to_char(at.min(rope.len_utf16_cu())),
                    )
                    .to_string();
                if let Ok(mut slot) = from_out.lock() {
                    *slot = (from, prefix);
                }
                let params = CompletionParams {
                    text_document_position: text_position(uri, position(state, offset)),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                    context: Some(CompletionContext {
                        trigger_kind: if trigger.is_some() {
                            CompletionTriggerKind::TRIGGER_CHARACTER
                        } else {
                            CompletionTriggerKind::INVOKED
                        },
                        trigger_character: trigger,
                    }),
                };
                supervisor
                    .start_request::<Completion>(params)
                    .map_err(lsp_error)
            })
            .await?
        else {
            return Ok(None);
        };
        let Some(response) = prepared.pending.wait(&token).await.map_err(lsp_error)? else {
            return Ok(None);
        };
        let items = match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        let (from, prefix) = from_box.lock().map(|s| s.clone()).unwrap_or_default();
        let id = prepared.service.next_id();
        let (views, total) = rank(&items, &prefix);
        prepared.service.store_completion(CompletionSession {
            id,
            doc: doc_id,
            from,
            resolved: vec![false; items.len()],
            items,
        });
        Ok(Some(ipc::CompletionList {
            session: id,
            from: to_u32(from),
            items: views,
            total,
        }))
    }

    /// Ranks a session's items for the prefix typed so far (no server call).
    pub async fn filter_completion(
        &self,
        session: u32,
        prefix: String,
    ) -> Result<Option<ipc::CompletionList>, IpcError> {
        let service = self.runtime()?.lsp.clone();
        blocking(move || {
            Ok(service.with_completion(session, |s| {
                let (items, total) = rank(&s.items, &prefix);
                ipc::CompletionList {
                    session,
                    from: to_u32(s.from),
                    items,
                    total,
                }
            }))
        })
        .await
    }

    /// Resolves an item (once) through `completionItem/resolve`.
    async fn resolved_item(
        &self,
        service: &Arc<LanguageService>,
        session: u32,
        index: u32,
    ) -> Result<lsp::CompletionItem, IpcError> {
        let (item, resolved) = service
            .with_completion(session, |s| {
                let i = index as usize;
                s.items.get(i).cloned().map(|item| (item, s.resolved[i]))
            })
            .flatten()
            .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "completion item is gone"))?;
        if resolved {
            return Ok(item);
        }
        let supports = service
            .capabilities()
            .and_then(|c| c.completion_provider.clone())
            .and_then(|p| p.resolve_provider)
            .unwrap_or(false);
        let Some(supervisor) = service.running().filter(|_| supports) else {
            return Ok(item);
        };
        // While the server loads the project it may answer ContentModified
        // (mapped to Cancelled); retry briefly. A failed resolve is never
        // cached, so accepting retries it and keeps the auto-import edits.
        let mut attempt = 0;
        let resolved = loop {
            match supervisor
                .request::<ResolveCompletionItem>(item.clone(), &CancellationToken::new())
                .await
            {
                Ok(resolved) => break resolved,
                Err(LspError::Cancelled) if attempt < RESOLVE_RETRIES => {
                    attempt += 1;
                    tokio::time::sleep(RESOLVE_RETRY_DELAY).await;
                }
                Err(err) => {
                    tracing::debug!(%err, "completionItem/resolve failed");
                    return Ok(item);
                }
            }
        };
        // While the project loads, typescript-language-server answers from
        // syntax only (no details, no auto-import edits): use the answer but
        // resolve again later instead of caching it.
        if service.status().state != ipc::ServerStateKind::Ready {
            return Ok(resolved);
        }
        service.with_completion(session, |s| {
            if let Some(slot) = s.items.get_mut(index as usize) {
                *slot = resolved.clone();
                s.resolved[index as usize] = true;
            }
        });
        Ok(resolved)
    }

    /// Detail and documentation of one item, resolved lazily.
    pub async fn completion_details(
        &self,
        session: u32,
        index: u32,
    ) -> Result<ipc::CompletionDetails, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let item = self.resolved_item(&service, session, index).await?;
        let registry = self.registry.clone();
        blocking(move || {
            Ok(ipc::CompletionDetails {
                detail: item.detail.clone(),
                html: documentation_html(item.documentation.as_ref(), &registry),
            })
        })
        .await
    }

    /// The edits for accepting an item with the caret at `caret` (version
    /// `version`): resolved first, so auto-imports come along.
    pub async fn accept_completion(
        &self,
        doc_id: DocId,
        version: u32,
        session: u32,
        index: u32,
        caret: u32,
    ) -> Result<ipc::DocumentEdit, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let item = self.resolved_item(&service, session, index).await?;
        let from = service
            .with_completion(session, |s| s.from)
            .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "completion session is gone"))?;
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let state = handle.lock()?;
            check_version(&state, version)?;
            accept_edit(&state.doc, &item, from, to_usize(caret))
                .map_err(|e| IpcError::new(IpcErrorKind::Internal, e))
        })
        .await
    }

    // Signature help and usages

    /// The signature of the call around `offset`, if any.
    pub async fn signature_help(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
        trigger: Option<String>,
    ) -> Result<Option<ipc::SignatureView>, IpcError> {
        let token = self.runtime()?.lsp.token("signature", doc_id);
        let Some(prepared) = self
            .prepare(doc_id, Some(version), move |supervisor, state, uri| {
                let params = SignatureHelpParams {
                    context: Some(SignatureHelpContext {
                        trigger_kind: if trigger.is_some() {
                            SignatureHelpTriggerKind::TRIGGER_CHARACTER
                        } else {
                            SignatureHelpTriggerKind::INVOKED
                        },
                        trigger_character: trigger,
                        is_retrigger: false,
                        active_signature_help: None,
                    }),
                    text_document_position_params: text_position(uri, position(state, offset)),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                supervisor
                    .start_request::<SignatureHelpRequest>(params)
                    .map_err(lsp_error)
            })
            .await?
        else {
            return Ok(None);
        };
        let Some(help) = prepared.pending.wait(&token).await.map_err(lsp_error)? else {
            return Ok(None);
        };
        let registry = self.registry.clone();
        blocking(move || {
            let count = to_u32(help.signatures.len());
            let active = help
                .active_signature
                .unwrap_or(0)
                .min(count.saturating_sub(1));
            let Some(sig) = help.signatures.get(active as usize) else {
                return Ok(None);
            };
            let mut search_from = 0;
            let parameters: Vec<ipc::TextRange> = sig
                .parameters
                .iter()
                .flatten()
                .map(|p| match &p.label {
                    ParameterLabel::LabelOffsets([a, b]) => ipc::TextRange { from: *a, to: *b },
                    ParameterLabel::Simple(name) => {
                        let found = sig.label[search_from..].find(name.as_str());
                        let start = found.map_or(0, |i| search_from + i);
                        search_from = start + name.len();
                        let utf16 = |byte: usize| to_u32(sig.label[..byte].encode_utf16().count());
                        ipc::TextRange {
                            from: utf16(start),
                            to: utf16(start + name.len()),
                        }
                    }
                })
                .collect();
            let active_parameter = sig.active_parameter.or(help.active_parameter);
            let param_doc = active_parameter
                .and_then(|i| sig.parameters.as_ref()?.get(i as usize))
                .and_then(|p| documentation_html(p.documentation.as_ref(), &registry));
            let html = match (
                param_doc,
                documentation_html(sig.documentation.as_ref(), &registry),
            ) {
                (Some(p), Some(s)) => Some(format!("{p}{s}")),
                (p, s) => p.or(s),
            };
            Ok(Some(ipc::SignatureView {
                label: sig.label.clone(),
                parameters,
                active_parameter,
                active_signature: active,
                signatures: count,
                html,
            }))
        })
        .await
    }

    /// References of the symbol at `offset`, grouped by file.
    pub async fn find_usages(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
    ) -> Result<ipc::UsageResults, IpcError> {
        let token = self.runtime()?.lsp.token("usages", doc_id);
        let symbol = Arc::new(std::sync::Mutex::new(String::new()));
        let symbol_out = symbol.clone();
        let prepared = self
            .prepare_required(doc_id, version, move |supervisor, state, uri| {
                if let Ok(mut slot) = symbol_out.lock() {
                    *slot = word_at(state.doc.rope(), to_usize(offset)).2;
                }
                let params = ReferenceParams {
                    text_document_position: text_position(uri, position(state, offset)),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                    context: ReferenceContext {
                        include_declaration: true,
                    },
                };
                supervisor
                    .start_request::<References>(params)
                    .map_err(lsp_error)
            })
            .await?;
        let locations = prepared
            .pending
            .wait(&token)
            .await
            .map_err(lsp_error)?
            .unwrap_or_default();
        let symbol = symbol.lock().map(|s| s.clone()).unwrap_or_default();
        let runtime = self.runtime()?;
        let open = self.open_docs();
        blocking(move || {
            let mut groups: Vec<ipc::UsageGroup> = Vec::new();
            let mut texts: HashMap<PathBuf, Option<Rope>> = HashMap::new();
            for location in locations {
                let Some(path) = uri_to_path(&location.uri) else {
                    continue;
                };
                let rope = texts.entry(path.clone()).or_insert_with(|| {
                    match open.iter().find(|d| d.path == path) {
                        Some(doc) => doc.lock().ok().map(|s| s.doc.rope().clone()),
                        None => ferrule_vfs::read_text(&path)
                            .ok()
                            .map(|f| Rope::from_str(&f.text)),
                    }
                });
                let range = location.range;
                let usage = ipc::Usage {
                    line: range.start.line,
                    column: range.start.character,
                    length: if range.end.line == range.start.line {
                        range.end.character.saturating_sub(range.start.character)
                    } else {
                        0
                    },
                    preview: rope
                        .as_ref()
                        .map(|r| line_text(r, range.start.line))
                        .unwrap_or_default(),
                };
                let relative = runtime.project.relativize(&path);
                let display = relative
                    .clone()
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                match groups.iter_mut().find(|g| g.path == display) {
                    Some(group) => group.usages.push(usage),
                    None => groups.push(ipc::UsageGroup {
                        path: display,
                        in_project: relative.is_some(),
                        usages: vec![usage],
                    }),
                }
            }
            for group in &mut groups {
                group.usages.sort_by_key(|u| (u.line, u.column));
            }
            groups.sort_by(|a, b| b.in_project.cmp(&a.in_project).then(a.path.cmp(&b.path)));
            Ok(ipc::UsageResults { symbol, groups })
        })
        .await
    }

    // Rename

    /// The symbol Rename would change at `offset` (`None`: not renameable).
    pub async fn prepare_rename(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
    ) -> Result<Option<ipc::RenameTarget>, IpcError> {
        let handle = self.doc(doc_id)?;
        let supports_prepare = self
            .runtime()?
            .lsp
            .capabilities()
            .and_then(|c| c.rename_provider.clone())
            .is_some_and(
                |p| matches!(p, OneOf::Right(options) if options.prepare_provider == Some(true)),
            );
        let fallback = {
            let h = handle.clone();
            move || -> Result<Option<ipc::RenameTarget>, IpcError> {
                let state = h.lock()?;
                let (from, to, text) = word_at(state.doc.rope(), to_usize(offset));
                Ok((from < to).then(|| ipc::RenameTarget {
                    from: to_u32(from),
                    to: to_u32(to),
                    placeholder: text,
                }))
            }
        };
        if !supports_prepare {
            return blocking(fallback).await;
        }
        let prepared = self
            .prepare_required(doc_id, version, move |supervisor, state, uri| {
                supervisor
                    .start_request::<PrepareRenameRequest>(text_position(
                        uri,
                        position(state, offset),
                    ))
                    .map_err(lsp_error)
            })
            .await?;
        let response = prepared
            .pending
            .wait(&CancellationToken::new())
            .await
            .map_err(lsp_error)?;
        blocking(move || {
            let state = handle.lock()?;
            let doc = &state.doc;
            let offset_of = |p: Position| {
                to_u32(doc.offset_of(LinePos::new(p.line as usize, p.character as usize)))
            };
            let slice = |from: u32, to: u32| {
                doc.rope()
                    .slice(
                        doc.rope().utf16_cu_to_char(from as usize)
                            ..doc.rope().utf16_cu_to_char(to as usize),
                    )
                    .to_string()
            };
            Ok(match response {
                None => None,
                Some(PrepareRenameResponse::Range(range)) => {
                    let (from, to) = (offset_of(range.start), offset_of(range.end));
                    Some(ipc::RenameTarget {
                        from,
                        to,
                        placeholder: slice(from, to),
                    })
                }
                Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) => {
                    Some(ipc::RenameTarget {
                        from: offset_of(range.start),
                        to: offset_of(range.end),
                        placeholder,
                    })
                }
                Some(PrepareRenameResponse::DefaultBehavior { .. }) => {
                    let (from, to, text) = word_at(doc.rope(), to_usize(offset));
                    (from < to).then(|| ipc::RenameTarget {
                        from: to_u32(from),
                        to: to_u32(to),
                        placeholder: text,
                    })
                }
            })
        })
        .await
    }

    /// Asks the server what renaming the symbol at `offset` to `new_name`
    /// changes; nothing is changed until [`Session::apply_refactoring`].
    pub async fn rename_preview(
        &self,
        doc_id: DocId,
        version: u32,
        offset: u32,
        new_name: String,
    ) -> Result<ipc::RefactoringPreview, IpcError> {
        let prepared = self
            .prepare_required(doc_id, version, move |supervisor, state, uri| {
                let params = RenameParams {
                    text_document_position: text_position(uri, position(state, offset)),
                    new_name,
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                supervisor
                    .start_request::<Rename>(params)
                    .map_err(lsp_error)
            })
            .await?;
        let edit = prepared
            .pending
            .wait(&CancellationToken::new())
            .await
            .map_err(lsp_error)?
            .ok_or_else(|| {
                IpcError::new(IpcErrorKind::Conflict, "this symbol cannot be renamed")
            })?;
        self.preview(prepared.service, &edit).await
    }

    async fn preview(
        &self,
        service: Arc<LanguageService>,
        edit: &WorkspaceEdit,
    ) -> Result<ipc::RefactoringPreview, IpcError> {
        let (files, skipped) = edits::collect(edit);
        let session = self.self_arc()?;
        blocking(move || {
            let expected = edits::snapshot(&session, &files)?;
            let preview_files: Vec<ipc::PreviewFile> = files
                .iter()
                .map(|f| ipc::PreviewFile {
                    path: session
                        .relative_path(&f.path)
                        .unwrap_or_else(|| f.path.to_string_lossy().into_owned()),
                    count: to_u32(f.edits.len()),
                    read_only: is_library(&session, &f.path),
                })
                .collect();
            let total = preview_files.iter().map(|f| f.count).sum();
            let id = service.next_id();
            service.store_refactoring(
                id,
                Refactoring {
                    files,
                    expected,
                    skipped,
                },
            );
            Ok(ipc::RefactoringPreview {
                id,
                files: preview_files,
                total,
            })
        })
        .await
    }

    /// Applies a previewed refactoring: open documents get edits for the
    /// view (one undo step each), closed files are rewritten unless they
    /// changed since the preview.
    pub async fn apply_refactoring(&self, id: u32) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let refactoring = service
            .take_refactoring(id)
            .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "the preview is gone"))?;
        let session = self.self_arc()?;
        blocking(move || {
            Ok(edits::apply(
                &session,
                refactoring.files,
                Some(&refactoring.expected),
                refactoring.skipped,
            ))
        })
        .await
    }

    /// Applies a workspace edit at once (code actions, formatting).
    async fn apply_now(
        &self,
        files: Vec<FileEdits>,
        skipped: Vec<ipc::SkippedFile>,
    ) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let session = self.self_arc()?;
        blocking(move || Ok(edits::apply(&session, files, None, skipped))).await
    }

    // Code actions

    /// Quick fixes and refactorings for the range `from..to`.
    pub async fn code_actions(
        &self,
        doc_id: DocId,
        version: u32,
        from: u32,
        to: u32,
    ) -> Result<Vec<ipc::CodeActionView>, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let actions = self
            .request_code_actions(doc_id, version, Some((from, to)), None)
            .await?;
        Ok(actions
            .into_iter()
            .map(|action| {
                let id = service.next_id();
                let view = match &action {
                    CodeActionOrCommand::Command(cmd) => ipc::CodeActionView {
                        id,
                        title: cmd.title.clone(),
                        kind: String::new(),
                        preferred: false,
                        disabled: None,
                    },
                    CodeActionOrCommand::CodeAction(a) => ipc::CodeActionView {
                        id,
                        title: a.title.clone(),
                        kind: a
                            .kind
                            .as_ref()
                            .map(|k| k.as_str().to_owned())
                            .unwrap_or_default(),
                        preferred: a.is_preferred.unwrap_or(false),
                        disabled: a.disabled.as_ref().map(|d| d.reason.clone()),
                    },
                };
                service.store_code_action(id, action);
                view
            })
            .collect())
    }

    async fn request_code_actions(
        &self,
        doc_id: DocId,
        version: u32,
        range: Option<(u32, u32)>,
        only: Option<Vec<CodeActionKind>>,
    ) -> Result<Vec<CodeActionOrCommand>, IpcError> {
        let handle = self.doc(doc_id)?;
        let service = self.runtime()?.lsp.clone();
        let diagnostics = service.diagnostics_for(&handle.rel_path);
        let prepared = self
            .prepare_required(doc_id, version, move |supervisor, state, uri| {
                let (from, to) = range.unwrap_or((0, to_u32(state.doc.len_utf16())));
                let range = lsp::Range::new(position(state, from), position(state, to));
                let overlapping = diagnostics
                    .into_iter()
                    .filter(|d| d.range.start <= range.end && range.start <= d.range.end)
                    .collect();
                let params = CodeActionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    range,
                    context: CodeActionContext {
                        diagnostics: overlapping,
                        only,
                        trigger_kind: Some(CodeActionTriggerKind::INVOKED),
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                };
                supervisor
                    .start_request::<CodeActionRequest>(params)
                    .map_err(lsp_error)
            })
            .await?;
        Ok(prepared
            .pending
            .wait(&CancellationToken::new())
            .await
            .map_err(lsp_error)?
            .unwrap_or_default())
    }

    /// Applies a code action listed by [`Session::code_actions`].
    pub async fn apply_code_action(&self, id: u32) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let action = service
            .take_code_action(id)
            .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "the code action is gone"))?;
        self.run_code_action(&service, action).await
    }

    async fn run_code_action(
        &self,
        service: &Arc<LanguageService>,
        action: CodeActionOrCommand,
    ) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let supervisor = service
            .running()
            .ok_or_else(|| lsp_error(LspError::NotRunning))?;
        let (mut edit, command) = match action {
            CodeActionOrCommand::Command(cmd) => (None, Some(cmd)),
            CodeActionOrCommand::CodeAction(mut action) => {
                if action.edit.is_none()
                    && action.data.is_some()
                    && let Ok(resolved) = supervisor
                        .request::<CodeActionResolveRequest>(
                            action.clone(),
                            &CancellationToken::new(),
                        )
                        .await
                {
                    action = resolved;
                }
                (action.edit, action.command)
            }
        };
        let mut edits: Vec<WorkspaceEdit> = edit.take().into_iter().collect();
        if let Some(command) = command {
            // The server answers with workspace/applyEdit while this runs.
            let _command_guard = service.command_lock.lock().await;
            service.begin_capture();
            let token = CancellationToken::new();
            let run = supervisor.request::<ExecuteCommand>(
                ExecuteCommandParams {
                    command: command.command,
                    arguments: command.arguments.unwrap_or_default(),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                },
                &token,
            );
            let outcome = tokio::time::timeout(COMMAND_TIMEOUT, run).await;
            edits.extend(service.end_capture());
            match outcome {
                Err(_) => {
                    return Err(IpcError::new(
                        IpcErrorKind::Internal,
                        "the server did not finish the action in time",
                    ));
                }
                Ok(Err(err)) => return Err(lsp_error(err)),
                Ok(Ok(_)) => {}
            }
        }
        let mut files: Vec<FileEdits> = Vec::new();
        let mut skipped = Vec::new();
        for edit in &edits {
            let (more, more_skipped) = edits::collect(edit);
            for file in more {
                match files.iter_mut().find(|f| f.path == file.path) {
                    Some(existing) => existing.edits.extend(file.edits),
                    None => files.push(file),
                }
            }
            skipped.extend(more_skipped);
        }
        self.apply_now(files, skipped).await
    }

    /// Organize Imports from the server (`source.organizeImports`).
    pub async fn organize_imports(
        &self,
        doc_id: DocId,
        version: u32,
    ) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let service = self.runtime()?.lsp.clone();
        let actions = self
            .request_code_actions(
                doc_id,
                version,
                None,
                Some(vec![CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
            )
            .await?;
        let Some(action) = actions.into_iter().next() else {
            return Ok(ipc::WorkspaceEditResult {
                open_documents: Vec::new(),
                files_written: Vec::new(),
                skipped: Vec::new(),
            });
        };
        self.run_code_action(&service, action).await
    }

    /// Sorts the import block in-process (oxc), without the server.
    pub async fn sort_imports(
        &self,
        doc_id: DocId,
        version: u32,
    ) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let state = handle.lock()?;
            check_version(&state, version)?;
            let text = state.doc.text();
            let edits = ferrule_lang_js::sort_imports(&text, &handle.path)
                .map(|edit| {
                    vec![ipc::TextEdit {
                        from: to_u32(state.doc.byte_to_utf16(edit.start)),
                        to: to_u32(state.doc.byte_to_utf16(edit.end)),
                        insert: edit.text,
                    }]
                })
                .unwrap_or_default();
            let open_documents = if edits.is_empty() {
                Vec::new()
            } else {
                vec![ipc::DocumentReplace {
                    doc_id,
                    version,
                    edits,
                }]
            };
            Ok(ipc::WorkspaceEditResult {
                open_documents,
                files_written: Vec::new(),
                skipped: Vec::new(),
            })
        })
        .await
    }

    /// Reformat Code with the server's document formatting.
    pub async fn format_document(
        &self,
        doc_id: DocId,
        version: u32,
    ) -> Result<ipc::WorkspaceEditResult, IpcError> {
        let prepared = self
            .prepare_required(doc_id, version, move |supervisor, state, uri| {
                let params = DocumentFormattingParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    options: FormattingOptions {
                        tab_size: state.tab_width(),
                        insert_spaces: !state.indent_unit().starts_with('\t'),
                        ..FormattingOptions::default()
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                supervisor
                    .start_request::<Formatting>(params)
                    .map_err(lsp_error)
            })
            .await?;
        let edits = prepared
            .pending
            .wait(&CancellationToken::new())
            .await
            .map_err(lsp_error)?
            .unwrap_or_default();
        let files = vec![FileEdits {
            path: prepared.handle.path.clone(),
            edits,
        }];
        self.apply_now(files, Vec::new()).await
    }
}

/// Expectations are only used by previews; keep the type in view here.
#[allow(dead_code)]
fn _expect_type(_: Expect) {}
