//! LSP base protocol framing: `Content-Length: N\r\n\r\n` followed by `N`
//! bytes of UTF-8 JSON.

use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest message accepted from a server (a guard against garbage input).
const MAX_CONTENT_LENGTH: usize = 256 * 1024 * 1024;
/// Longest header line accepted.
const MAX_HEADER_LINE: u64 = 8 * 1024;

/// Reads one message body. `Ok(None)` at a clean end of stream (before any
/// header byte); an end of stream inside a message is an error.
pub async fn read_message<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    let mut saw_header = false;
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut *reader)
            .take(MAX_HEADER_LINE)
            .read_line(&mut line)
            .await?;
        if read == 0 {
            if saw_header {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ended inside a message header",
                ));
            }
            return Ok(None);
        }
        if !line.ends_with('\n') {
            return Err(invalid(format!(
                "header line too long or unterminated: {:?}",
                truncate(&line)
            )));
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if !saw_header {
                // Tolerate stray blank lines between messages.
                continue;
            }
            break;
        }
        saw_header = true;
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| invalid(format!("malformed header {:?}", truncate(header))))?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            let length: usize = value
                .trim()
                .parse()
                .map_err(|_| invalid(format!("bad Content-Length {:?}", value.trim())))?;
            if length > MAX_CONTENT_LENGTH {
                return Err(invalid(format!("Content-Length {length} is too large")));
            }
            content_length = Some(length);
        }
        // Other headers (Content-Type) are allowed and ignored.
    }
    let length = content_length.ok_or_else(|| invalid("message without Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| invalid("message body is not UTF-8"))
}

/// Writes one message and flushes.
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, body: &str) -> io::Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body.as_bytes()).await?;
    writer.flush().await
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn truncate(text: &str) -> &str {
    let end = text
        .char_indices()
        .nth(80)
        .map_or(text.len(), |(index, _)| index);
    &text[..end]
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    async fn read_all(input: &[u8]) -> io::Result<Vec<String>> {
        let mut reader = BufReader::new(input);
        let mut messages = Vec::new();
        while let Some(message) = read_message(&mut reader).await? {
            messages.push(message);
        }
        Ok(messages)
    }

    #[tokio::test]
    async fn round_trips_messages_with_multibyte_text() {
        let mut out = Vec::new();
        write_message(&mut out, r#"{"a":"😀 漢字"}"#).await.unwrap();
        write_message(&mut out, "{}").await.unwrap();
        // Content-Length counts bytes, not characters.
        assert!(out.starts_with(b"Content-Length: 19\r\n\r\n"));
        assert_eq!(read_all(&out).await.unwrap(), [r#"{"a":"😀 漢字"}"#, "{}"]);
    }

    #[tokio::test]
    async fn accepts_other_headers_any_case_and_bare_newlines() {
        let input = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}\
                      Content-Length:2\n\n[]";
        assert_eq!(read_all(input).await.unwrap(), ["{}", "[]"]);
    }

    #[tokio::test]
    async fn reads_messages_split_across_reads() {
        let (mut client, server) = tokio::io::duplex(8);
        let writer = tokio::spawn(async move {
            write_message(&mut client, r#"{"long":"message body"}"#)
                .await
                .unwrap();
        });
        let mut reader = BufReader::new(server);
        let message = read_message(&mut reader).await.unwrap();
        assert_eq!(message.as_deref(), Some(r#"{"long":"message body"}"#));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_malformed_input() {
        assert!(read_all(b"Content-Length: x\r\n\r\n").await.is_err());
        assert!(read_all(b"Content-Type: a\r\n\r\n{}").await.is_err());
        assert!(read_all(b"garbage\r\n\r\n").await.is_err());
        assert!(read_all(b"Content-Length: 10\r\n\r\n{}").await.is_err());
        assert!(read_all(b"Content-Length: 2\r\n").await.is_err());
        assert!(
            read_all(b"Content-Length: 999999999999\r\n\r\n")
                .await
                .is_err()
        );
        assert!(read_all(b"").await.unwrap().is_empty());
    }
}
