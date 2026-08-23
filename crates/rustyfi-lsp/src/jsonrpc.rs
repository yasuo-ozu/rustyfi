//! Base-protocol framing and JSON-RPC message shapes.
//!
//! # Why this is hand-rolled
//!
//! The obvious alternative is `lsp-types` + `lsp-server` (rust-analyzer's,
//! both MIT). They were evaluated and declined, on weight:
//!
//! - The framing is `Content-Length: N\r\n\r\n<json>` and nothing else. That
//!   is the whole of the base protocol, and it is the ~60 lines below.
//! - This server implements eight methods, every one of whose payloads is a
//!   handful of scalars. `lsp-types` is a large crate of type definitions for
//!   a protocol surface ~50× the one in use here, and it pulls `url` — which
//!   brings `idna`, `unicode-bidi`, `tinyvec`, `percent-encoding` and friends
//!   — for a field this server treats as an opaque document key and never
//!   parses.
//! - `lsp-server` adds `crossbeam-channel` and a reader/writer thread pair,
//!   for a loop that is inherently sequential: read a message, answer it,
//!   read the next.
//!
//! Against that, `serde_json` is *already* in this workspace's dependency
//! graph (`rustyfi`, `rustyfi-pdf`), so the hand-rolled route adds **no new
//! crate at all**, and the whole protocol layer stays under 200 lines. The
//! workspace is deliberately lean and there is no async runtime here for the
//! same reason.
//!
//! The trade is real and worth naming: `serde_json::Value` params are checked
//! at use, not by the type system, so a shape mistake is a runtime `None`
//! rather than a compile error. That is an acceptable price at eight methods;
//! it would not be at eighty, and if this server grows hover, completion and
//! rename, `lsp-types` becomes the right call.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// A decoded incoming message.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// A request: has both `method` and `id`, and must be answered.
    Request {
        /// The `id` to echo in the response. Any JSON value per JSON-RPC —
        /// kept as a `Value` rather than narrowed to an integer, because a
        /// client is entitled to send a string and echoing back the wrong
        /// type silently breaks its request matching.
        id: Value,
        /// The method name.
        method: String,
        /// The `params` member, or `Value::Null` if absent.
        params: Value,
    },
    /// A notification: `method`, no `id`, no answer.
    Notification {
        /// The method name.
        method: String,
        /// The `params` member, or `Value::Null` if absent.
        params: Value,
    },
    /// A response to a request *we* sent. This server sends none, so these
    /// are ignored; the variant exists so they are ignored deliberately
    /// rather than misread as a malformed request.
    Response,
}

/// JSON-RPC / LSP error codes this server can emit.
pub mod code {
    /// The request's method is not implemented.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The payload was not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// A request arrived that is not valid in the current state.
    pub const INVALID_REQUEST: i64 = -32600;
    /// A request arrived before `initialize`.
    pub const SERVER_NOT_INITIALIZED: i64 = -32002;
}

/// Read one framed message, or `Ok(None)` at a clean end of input.
///
/// Only `Content-Length` is interpreted; every other header (`Content-Type`,
/// which the spec allows and some clients send) is skipped. Header lines are
/// accepted with either `\r\n` or a bare `\n` terminator — the spec mandates
/// `\r\n`, but accepting `\n` costs nothing and makes a hand-written test
/// script or a `printf` debugging session work.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            // EOF. Clean only if it happened between messages; a truncated
            // header block is reported as an unexpected EOF.
            return match content_length {
                None => Ok(None),
                Some(_) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "end of input inside a message header",
                )),
            };
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // End of the header block.
        }
        if let Some(rest) = header_value(trimmed, "content-length") {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }

    let Some(len) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message header had no Content-Length",
        ));
    };
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    match serde_json::from_slice::<Value>(&body) {
        Ok(v) => Ok(Some(v)),
        // Surfaced as `InvalidData` so the caller can answer with a
        // `PARSE_ERROR` response rather than tearing the connection down.
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
    }
}

/// `name`'s value if `line` is that header. Header names are
/// case-insensitive per RFC 7230, which the LSP base protocol follows.
fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let (key, value) = line.split_once(':')?;
    key.trim().eq_ignore_ascii_case(name).then_some(value)
}

/// Classify a decoded message.
pub fn classify(mut msg: Value) -> Incoming {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    // Moved out, not cloned: for a `didChange` the params hold the entire
    // document text, so cloning them would copy the whole buffer once per
    // keystroke for nothing — `msg` is owned here and dropped straight after.
    let params = msg.get_mut("params").map(Value::take).unwrap_or(Value::Null);
    match (method, msg.get("id")) {
        (Some(method), Some(id)) => Incoming::Request {
            id: id.clone(),
            method,
            params,
        },
        (Some(method), None) => Incoming::Notification { method, params },
        (None, _) => Incoming::Response,
    }
}

/// Write one framed message.
///
/// Always emits `\r\n`, whatever was accepted on the way in, and flushes:
/// an editor waiting on a response must not be held up by a buffer.
pub fn write_message(out: &mut impl Write, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}

/// A successful response to `id`.
pub fn response(id: Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error response to `id`.
pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// A notification to the client.
pub fn notification(method: &str, params: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn reads_a_framed_message() {
        let bytes = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        let mut r = io::Cursor::new(bytes);
        let msg = read_message(&mut r).unwrap().unwrap();
        assert_eq!(msg["method"], "initialize");
        assert!(read_message(&mut r).unwrap().is_none(), "clean EOF after the last message");
    }

    #[test]
    fn reads_back_to_back_messages_without_losing_the_second() {
        let mut bytes = frame(r#"{"id":1,"method":"a"}"#);
        bytes.extend(frame(r#"{"id":2,"method":"b"}"#));
        let mut r = io::Cursor::new(bytes);
        assert_eq!(read_message(&mut r).unwrap().unwrap()["method"], "a");
        assert_eq!(read_message(&mut r).unwrap().unwrap()["method"], "b");
        assert!(read_message(&mut r).unwrap().is_none());
    }

    #[test]
    fn skips_other_headers_and_accepts_a_bare_lf() {
        let body = r#"{"method":"x"}"#;
        let raw = format!(
            "Content-Length: {}\nContent-Type: application/vscode-jsonrpc; charset=utf-8\n\n{body}",
            body.len()
        );
        let mut r = io::Cursor::new(raw.into_bytes());
        assert_eq!(read_message(&mut r).unwrap().unwrap()["method"], "x");
    }

    #[test]
    fn content_length_counts_bytes_not_characters() {
        // A body with multibyte content: a character-counting reader would
        // truncate it and then desynchronize the whole stream.
        let body = r#"{"text":"こんにちは"}"#;
        assert_ne!(body.len(), body.chars().count());
        let mut r = io::Cursor::new(frame(body));
        assert_eq!(read_message(&mut r).unwrap().unwrap()["text"], "こんにちは");
    }

    #[test]
    fn a_header_with_no_content_length_is_an_error() {
        let mut r = io::Cursor::new(b"Content-Type: x\r\n\r\n{}".to_vec());
        assert!(read_message(&mut r).is_err());
    }

    #[test]
    fn classify_separates_the_three_kinds() {
        let req = classify(serde_json::json!({"id": 1, "method": "m"}));
        assert!(matches!(req, Incoming::Request { .. }));
        let note = classify(serde_json::json!({"method": "m"}));
        assert!(matches!(note, Incoming::Notification { .. }));
        let resp = classify(serde_json::json!({"id": 1, "result": null}));
        assert!(matches!(resp, Incoming::Response));
    }

    #[test]
    fn a_string_id_survives_the_round_trip() {
        // JSON-RPC allows a string id; narrowing it to an integer would break
        // clients that use one.
        let req = classify(serde_json::json!({"id": "abc", "method": "m"}));
        let Incoming::Request { id, .. } = req else {
            panic!("expected a request")
        };
        assert_eq!(response(id, Value::Null)["id"], "abc");
    }

    #[test]
    fn written_frames_are_byte_counted_and_crlf_delimited() {
        let mut out = Vec::new();
        write_message(&mut out, &serde_json::json!({"a": "あ"})).unwrap();
        let text = String::from_utf8(out).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert_eq!(head, format!("Content-Length: {}", body.len()));
        assert_eq!(body, r#"{"a":"あ"}"#);
    }
}
