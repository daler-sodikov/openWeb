//! Search Everywhere, Recent Files and Find/Replace in Files.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use ferrule_ipc::{self as ipc, IpcError, IpcErrorKind, SearchItem, SearchTab};
use ferrule_search::{
    TextQuery, find_matches, fuzzy_match, match_path, replace_all, replacement_for,
};

use crate::convert::{action_to_ipc, to_u32, vfs_error};
use crate::session::{Session, blocking, lock};

/// Results per provider in the All tab, and in a single-provider tab.
const ALL_TAB_LIMITS: (usize, usize, usize) = (10, 10, 5);
const TAB_LIMIT: usize = 100;
/// Find in Files stops after this many matches.
const MAX_FIND_MATCHES: usize = 2000;
/// Larger files are skipped by Find/Replace in Files.
const MAX_SEARCH_FILE_SIZE: u64 = 4 * 1024 * 1024;

fn positions(p: &[usize]) -> Vec<u32> {
    p.iter().map(|&i| to_u32(i)).collect()
}

fn cancelled() -> IpcError {
    IpcError::new(IpcErrorKind::Cancelled, "superseded by a newer search")
}

impl Session {
    /// Search Everywhere: files, symbols and actions ranked by fuzzy score.
    /// An empty query shows recent files and all actions.
    pub async fn search_everywhere(
        &self,
        query: String,
        tab: SearchTab,
    ) -> Result<ipc::SearchResults, IpcError> {
        let runtime = self.runtime().ok();
        blocking(move || {
            let (file_limit, symbol_limit, action_limit) = match tab {
                SearchTab::All => ALL_TAB_LIMITS,
                SearchTab::Files => (TAB_LIMIT, 0, 0),
                SearchTab::Symbols => (0, TAB_LIMIT, 0),
                SearchTab::Actions => (0, 0, TAB_LIMIT),
            };
            let query = query.trim();
            let mut items = Vec::new();
            let mut indexing = false;

            if let Some(runtime) = &runtime {
                let guard = runtime
                    .index
                    .read()
                    .map_err(|_| IpcError::new(IpcErrorKind::Internal, "index lock poisoned"))?;
                indexing = guard.is_none();
                if file_limit > 0 {
                    if query.is_empty() {
                        let recent = lock(&runtime.recent)?.clone();
                        items.extend(recent.into_iter().take(file_limit).map(|path| {
                            SearchItem::File {
                                path,
                                positions: Vec::new(),
                            }
                        }));
                    } else if let Some(index) = guard.as_ref() {
                        let mut files: Vec<_> = index
                            .files()
                            .filter_map(|f| match_path(query, &f.path).map(|m| (m, &f.path)))
                            .collect();
                        files.sort_by_key(|(m, p)| (Reverse(m.score), p.len(), (*p).clone()));
                        items.extend(files.into_iter().take(file_limit).map(|(m, p)| {
                            SearchItem::File {
                                path: p.clone(),
                                positions: positions(&m.positions),
                            }
                        }));
                    }
                }
                if symbol_limit > 0
                    && !query.is_empty()
                    && let Some(index) = guard.as_ref()
                {
                    let mut symbols: Vec<_> = index
                        .files()
                        .flat_map(|f| f.symbols.iter().map(move |s| (f, s)))
                        .filter_map(|(f, s)| fuzzy_match(query, &s.name).map(|m| (m, f, s)))
                        .collect();
                    symbols.sort_by_key(|(m, f, s)| {
                        (Reverse(m.score), s.name.len(), f.path.clone(), s.line)
                    });
                    items.extend(symbols.into_iter().take(symbol_limit).map(|(m, f, s)| {
                        SearchItem::Symbol {
                            name: s.name.clone(),
                            kind: s.kind.clone(),
                            path: f.path.clone(),
                            line: s.line,
                            column: s.column,
                            positions: positions(&m.positions),
                        }
                    }));
                }
            }

            if action_limit > 0 {
                let mut actions: Vec<_> = ferrule_settings::ALL_ACTIONS
                    .iter()
                    .filter_map(|&a| {
                        let name = ferrule_settings::action_name(a);
                        fuzzy_match(query, name).map(|m| (m, a, name))
                    })
                    .collect();
                if !query.is_empty() {
                    actions.sort_by_key(|(m, _, name)| (Reverse(m.score), name.len()));
                }
                items.extend(actions.into_iter().take(action_limit).map(|(m, a, name)| {
                    SearchItem::Action {
                        action: action_to_ipc(a),
                        name: name.to_owned(),
                        positions: positions(&m.positions),
                    }
                }));
            }
            Ok(ipc::SearchResults { items, indexing })
        })
        .await
    }

    /// Recently opened files of the project, most recent first.
    pub async fn recent_files(&self) -> Result<Vec<String>, IpcError> {
        let runtime = self.runtime()?;
        Ok(lock(&runtime.recent)?.clone())
    }

    /// Unsaved text of open documents, by project-relative path.
    fn open_texts(&self) -> Result<HashMap<String, (ipc::DocId, u32, String)>, IpcError> {
        let docs: Vec<_> = lock(&self.state)?.docs.values().cloned().collect();
        let mut out = HashMap::new();
        for doc in docs {
            let state = doc.lock()?;
            out.insert(
                doc.rel_path.clone(),
                (doc.id, state.doc.version(), state.doc.text()),
            );
        }
        Ok(out)
    }

    /// Find in Files over the indexed files; open documents are searched in
    /// their current (possibly unsaved) state. A newer request cancels this one.
    pub async fn find_in_files(
        &self,
        search: ipc::TextSearch,
    ) -> Result<ipc::FindResults, IpcError> {
        let generation = self.find_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let runtime = self.runtime()?;
        let query = text_query(&search);
        let re = query
            .compile()
            .map_err(|e| IpcError::new(IpcErrorKind::InvalidArgument, e.to_string()))?;
        let open = self.open_texts()?;
        let session = self.generation_handle();
        blocking(move || {
            let guard = runtime
                .index
                .read()
                .map_err(|_| IpcError::new(IpcErrorKind::Internal, "index lock poisoned"))?;
            let Some(index) = guard.as_ref() else {
                return Ok(ipc::FindResults {
                    matches: Vec::new(),
                    truncated: false,
                    files_searched: 0,
                    indexing: true,
                });
            };
            let mut matches = Vec::new();
            let mut files_searched = 0u32;
            let mut truncated = false;
            for file in index.files() {
                if session.load(Ordering::SeqCst) != generation {
                    return Err(cancelled());
                }
                if search.pattern.is_empty() {
                    break;
                }
                let text = match open.get(&file.path) {
                    Some((_, _, text)) => text.clone(),
                    None => {
                        if file.size > MAX_SEARCH_FILE_SIZE {
                            continue;
                        }
                        match runtime
                            .project
                            .resolve(&file.path)
                            .ok()
                            .and_then(|abs| ferrule_vfs::read_text(&abs).ok())
                        {
                            Some(f) => f.text,
                            None => continue,
                        }
                    }
                };
                files_searched += 1;
                let remaining = MAX_FIND_MATCHES - matches.len();
                let found = find_matches(&re, &text, remaining + 1);
                if found.len() > remaining {
                    truncated = true;
                }
                for m in found.into_iter().take(remaining) {
                    let replacement = search
                        .replacement
                        .as_deref()
                        .map(|r| replacement_for(&re, &query, &text, &m, r));
                    matches.push(ipc::FindMatch {
                        path: file.path.clone(),
                        line: to_u32(m.line),
                        line_text: m.line_text,
                        column_start: to_u32(m.column_start),
                        column_end: to_u32(m.column_end),
                        replacement,
                    });
                }
                if truncated {
                    break;
                }
            }
            Ok(ipc::FindResults {
                matches,
                truncated,
                files_searched,
                indexing: false,
            })
        })
        .await
    }

    /// Replace in Files. Closed files are rewritten atomically (keeping
    /// line endings and BOM); edits for open documents are returned for the
    /// view to apply, so they go through the normal edit and undo path.
    pub async fn replace_in_files(
        &self,
        search: ipc::TextSearch,
        replacement: String,
    ) -> Result<ipc::ReplaceResult, IpcError> {
        self.find_counter.fetch_add(1, Ordering::SeqCst);
        let runtime = self.runtime()?;
        let query = text_query(&search);
        let re = query
            .compile()
            .map_err(|e| IpcError::new(IpcErrorKind::InvalidArgument, e.to_string()))?;
        if search.pattern.is_empty() {
            return Err(IpcError::new(
                IpcErrorKind::InvalidArgument,
                "the search text is empty",
            ));
        }
        let open = self.open_texts()?;
        blocking(move || {
            let paths: Vec<(String, u64)> = {
                let guard = runtime
                    .index
                    .read()
                    .map_err(|_| IpcError::new(IpcErrorKind::Internal, "index lock poisoned"))?;
                let index = guard.as_ref().ok_or_else(|| {
                    IpcError::new(IpcErrorKind::Conflict, "the project is still being indexed")
                })?;
                index.files().map(|f| (f.path.clone(), f.size)).collect()
            };
            let mut result = ipc::ReplaceResult {
                files_written: 0,
                replacements: 0,
                open_documents: Vec::new(),
            };
            for (path, size) in paths {
                if let Some((doc_id, version, text)) = open.get(&path) {
                    let (_, edits) = replace_all(&re, &query, text, &replacement);
                    if !edits.is_empty() {
                        result.replacements += to_u32(edits.len());
                        result.open_documents.push(ipc::DocumentReplace {
                            doc_id: *doc_id,
                            version: *version,
                            edits: edits
                                .into_iter()
                                .map(|e| ipc::TextEdit {
                                    from: to_u32(e.from),
                                    to: to_u32(e.to),
                                    insert: e.insert,
                                })
                                .collect(),
                        });
                    }
                    continue;
                }
                if size > MAX_SEARCH_FILE_SIZE {
                    continue;
                }
                let Ok(abs) = runtime.project.resolve(&path) else {
                    continue;
                };
                let Ok(file) = ferrule_vfs::read_text(&abs) else {
                    continue;
                };
                let (new_text, edits) = replace_all(&re, &query, &file.text, &replacement);
                if edits.is_empty() {
                    continue;
                }
                let bytes = ferrule_vfs::encode(&new_text, file.line_ending, file.bom);
                ferrule_vfs::write_atomic(&abs, &bytes)
                    .map_err(|e| vfs_error(ferrule_vfs::VfsError::Io(e)))?;
                result.files_written += 1;
                result.replacements += to_u32(edits.len());
            }
            tracing::info!(
                files = result.files_written,
                open = result.open_documents.len(),
                replacements = result.replacements,
                "replace in files"
            );
            Ok(result)
        })
        .await
    }

    fn generation_handle(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.find_counter.clone()
    }
}

fn text_query(search: &ipc::TextSearch) -> TextQuery {
    TextQuery {
        pattern: search.pattern.clone(),
        regex: search.regex,
        case_sensitive: search.case_sensitive,
        whole_word: search.whole_word,
    }
}
