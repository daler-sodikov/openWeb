//! A scripted language server for tests; not shipped. It speaks LSP over
//! stdio with its own blocking framing code and its own UTF-16 text model, so
//! the client is checked against an independent implementation.
//!
//! - `initialize`: incremental sync, hover and definition. Answers with the
//!   position encoding in `FAKE_LSP_ENCODING` (default `utf-16`). With
//!   `FAKE_LSP_FAIL_START` set it exits with code 3 instead.
//! - After `initialized`: one work-done progress ("Indexing fake project")
//!   begins and ends.
//! - Documents are rebuilt from incremental changes. After every open or
//!   change, an error diagnostic is published for each `error` in the text
//!   and a warning for each `warn`.
//! - `fake/text {uri}` → `{ text, version }`; `fake/log` → the methods
//!   received so far.
//! - `textDocument/hover` → `**word**` for the word at the position. The word
//!   `slow` is never answered until its `$/cancelRequest` arrives (then
//!   `RequestCancelled`).
//! - `textDocument/definition` → the first whole-word occurrence of the word
//!   in each open document.
//! - `fake/crash` (notification) exits with code 1; `exit` with code 0.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};

use serde_json::{Value, json};

struct Doc {
    text: String,
    version: i64,
}

struct Server {
    out: io::Stdout,
    docs: BTreeMap<String, Doc>,
    log: Vec<String>,
    slow: Vec<Value>,
}

fn main() {
    if std::env::var_os("FAKE_LSP_FAIL_START").is_some() {
        std::process::exit(3);
    }
    let _ = writeln!(io::stderr(), "fake-lsp-server started");
    let mut server = Server {
        out: io::stdout(),
        docs: BTreeMap::new(),
        log: Vec::new(),
        slow: Vec::new(),
    };
    let mut input = BufReader::new(io::stdin());
    // Stops when stdin closes or a message cannot be read.
    while let Ok(Some(message)) = read(&mut input) {
        if server.handle(&message).is_err() {
            break;
        }
    }
}

fn read(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        return Err(io::Error::other("missing Content-Length"));
    };
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

impl Server {
    fn send(&mut self, message: &Value) -> io::Result<()> {
        let body = message.to_string();
        let mut out = self.out.lock();
        write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
        out.flush()
    }

    fn reply(&mut self, id: &Value, result: Value) -> io::Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn handle(&mut self, message: &Value) -> io::Result<()> {
        let method = message["method"].as_str().unwrap_or("").to_owned();
        let id = message.get("id").cloned();
        let params = &message["params"];
        if !method.is_empty() {
            self.log.push(method.clone());
        }
        match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                let encoding =
                    std::env::var("FAKE_LSP_ENCODING").unwrap_or_else(|_| "utf-16".into());
                self.reply(
                    &id,
                    json!({
                        "capabilities": {
                            "positionEncoding": encoding,
                            "textDocumentSync": { "openClose": true, "change": 2, "save": true },
                            "hoverProvider": true,
                            "definitionProvider": true
                        },
                        "serverInfo": { "name": "fake-lsp-server", "version": "1.0" }
                    }),
                )
            }
            ("initialized", None) => {
                self.send(&json!({ "jsonrpc": "2.0", "id": "progress-1",
                    "method": "window/workDoneProgress/create", "params": { "token": "index" } }))?;
                self.send(&json!({ "jsonrpc": "2.0", "method": "$/progress",
                    "params": { "token": "index", "value": { "kind": "begin", "title": "Indexing fake project" } } }))?;
                self.send(&json!({ "jsonrpc": "2.0", "method": "$/progress",
                    "params": { "token": "index", "value": { "kind": "end" } } }))
            }
            ("textDocument/didOpen", None) => {
                let doc = &params["textDocument"];
                let uri = doc["uri"].as_str().unwrap_or("").to_owned();
                self.docs.insert(
                    uri.clone(),
                    Doc {
                        text: doc["text"].as_str().unwrap_or("").to_owned(),
                        version: doc["version"].as_i64().unwrap_or(0),
                    },
                );
                self.publish(&uri)
            }
            ("textDocument/didChange", None) => {
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                if let Some(doc) = self.docs.get_mut(&uri) {
                    doc.version = params["textDocument"]["version"].as_i64().unwrap_or(0);
                    for change in params["contentChanges"].as_array().into_iter().flatten() {
                        let text = change["text"].as_str().unwrap_or("");
                        match change.get("range") {
                            Some(range) => {
                                let start = offset(&doc.text, &range["start"]);
                                let end = offset(&doc.text, &range["end"]);
                                doc.text.replace_range(start..end, text);
                            }
                            None => doc.text = text.to_owned(),
                        }
                    }
                }
                self.publish(&uri)
            }
            ("textDocument/didClose", None) => {
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                self.docs.remove(&uri);
                self.send(
                    &json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": [] } }),
                )
            }
            ("textDocument/hover", Some(id)) => {
                let word = self.word_at(params);
                if word == "slow" {
                    self.slow.push(id);
                    return Ok(());
                }
                let result = if word.is_empty() {
                    Value::Null
                } else {
                    json!({ "contents": { "kind": "markdown", "value": format!("**{word}**") } })
                };
                self.reply(&id, result)
            }
            ("textDocument/definition", Some(id)) => {
                let word = self.word_at(params);
                let locations: Vec<Value> = self
                    .docs
                    .iter()
                    .filter_map(|(uri, doc)| {
                        let (line, column) = find_word(&doc.text, &word)?;
                        let end = column + word.encode_utf16().count();
                        Some(json!({ "uri": uri, "range": {
                            "start": { "line": line, "character": column },
                            "end": { "line": line, "character": end } } }))
                    })
                    .collect();
                self.reply(&id, Value::Array(locations))
            }
            ("$/cancelRequest", None) => {
                let target = params["id"].clone();
                if let Some(index) = self.slow.iter().position(|id| *id == target) {
                    let id = self.slow.remove(index);
                    self.send(&json!({ "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32800, "message": "cancelled" } }))?;
                }
                Ok(())
            }
            ("fake/text", Some(id)) => {
                let uri = params["uri"].as_str().unwrap_or("");
                let result = self.docs.get(uri).map_or(
                    Value::Null,
                    |doc| json!({ "text": doc.text, "version": doc.version }),
                );
                self.reply(&id, result)
            }
            ("fake/log", Some(id)) => {
                let log = json!(self.log);
                self.reply(&id, log)
            }
            ("fake/crash", None) => std::process::exit(1),
            ("shutdown", Some(id)) => self.reply(&id, Value::Null),
            ("exit", None) => std::process::exit(0),
            // A response to one of our requests (progress creation).
            ("", Some(_)) => Ok(()),
            (_, Some(id)) => self.send(&json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("unknown method {method}") } })),
            _ => Ok(()),
        }
    }

    fn word_at(&self, params: &Value) -> String {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let Some(doc) = self.docs.get(uri) else {
            return String::new();
        };
        let at = offset(&doc.text, &params["position"]);
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = doc.text[..at]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c))
            .last()
            .map_or(at, |(i, _)| i);
        let end = doc.text[at..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(doc.text.len(), |(i, _)| at + i);
        doc.text[start..end].to_owned()
    }

    fn publish(&mut self, uri: &str) -> io::Result<()> {
        let Some(doc) = self.docs.get(uri) else {
            return Ok(());
        };
        let mut diagnostics = Vec::new();
        for (word, severity) in [("error", 1), ("warn", 2)] {
            for (line_no, line) in doc.text.split('\n').enumerate() {
                for (byte, _) in line.match_indices(word) {
                    let start = line[..byte].encode_utf16().count();
                    diagnostics.push(json!({
                        "range": {
                            "start": { "line": line_no, "character": start },
                            "end": { "line": line_no, "character": start + word.len() }
                        },
                        "severity": severity,
                        "source": "fake",
                        "message": format!("found {word}")
                    }));
                }
            }
        }
        let version = doc.version;
        self.send(
            &json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "version": version, "diagnostics": diagnostics } }),
        )
    }
}

/// Byte offset of an LSP position (`\n` lines, UTF-16 columns, clamped).
fn offset(text: &str, position: &Value) -> usize {
    let line = usize::try_from(position["line"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
    let character =
        usize::try_from(position["character"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
    let mut start = 0;
    for (index, content) in text.split('\n').enumerate() {
        if index == line {
            let mut units = 0;
            for (byte, ch) in content.char_indices() {
                if units >= character {
                    return start + byte;
                }
                units += ch.len_utf16();
            }
            return start + content.len();
        }
        start += content.len() + 1;
    }
    text.len()
}

/// Line and UTF-16 column of the first whole-word occurrence of `word`.
fn find_word(text: &str, word: &str) -> Option<(usize, usize)> {
    if word.is_empty() {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    for (line_no, line) in text.split('\n').enumerate() {
        for (byte, _) in line.match_indices(word) {
            let before = line[..byte].chars().next_back();
            let after = line[byte + word.len()..].chars().next();
            if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
                return Some((line_no, line[..byte].encode_utf16().count()));
            }
        }
    }
    None
}
