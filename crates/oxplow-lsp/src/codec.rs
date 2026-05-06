//! LSP base-protocol framing.
//!
//! Each message is `Content-Length: <n>\r\n\r\n<n bytes of UTF-8 JSON>`.
//! `Content-Type` is optional and defaults to `application/vscode-jsonrpc;
//! charset=utf-8`; we ignore it on read and never emit it on write.

use std::io;

use bytes::{Buf, BytesMut};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("malformed header: {0}")]
    BadHeader(String),
    #[error("missing Content-Length")]
    MissingLength,
    #[error("body not utf-8: {0}")]
    BadBody(#[from] std::str::Utf8Error),
}

/// Length-prefixed JSON-RPC framed bytes.
///
/// Decoded payloads are raw JSON strings; the proxy parses them into
/// `serde_json::Value` so this codec stays JSON-agnostic.
pub struct LspCodec;

impl Decoder for LspCodec {
    type Item = String;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Find the end of the header block.
        let header_end = match find_double_crlf(src) {
            Some(idx) => idx,
            None => return Ok(None),
        };

        let header_bytes = &src[..header_end];
        let header = std::str::from_utf8(header_bytes)?;
        let mut content_length: Option<usize> = None;
        for line in header.split("\r\n") {
            if line.is_empty() {
                continue;
            }
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| CodecError::BadHeader(line.to_string()))?;
            if name.eq_ignore_ascii_case("Content-Length") {
                content_length =
                    Some(value.trim().parse().map_err(|e: std::num::ParseIntError| {
                        CodecError::BadHeader(e.to_string())
                    })?);
            }
        }
        let len = content_length.ok_or(CodecError::MissingLength)?;
        let total = header_end + 4 + len;
        if src.len() < total {
            return Ok(None);
        }

        src.advance(header_end + 4);
        let body = src.split_to(len);
        let s = std::str::from_utf8(&body)?.to_string();
        Ok(Some(s))
    }
}

impl Encoder<&str> for LspCodec {
    type Error = CodecError;

    fn encode(&mut self, item: &str, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let header = format!("Content-Length: {}\r\n\r\n", item.len());
        dst.extend_from_slice(header.as_bytes());
        dst.extend_from_slice(item.as_bytes());
        Ok(())
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Helper for tests that just want to write a framed JSON message
/// onto an `AsyncWrite` without going through `Framed`.
pub async fn write_framed<W: AsyncWriteExt + Unpin>(writer: &mut W, json: &str) -> io::Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::codec::Decoder;

    #[test]
    fn decode_returns_none_for_partial() {
        let mut buf = BytesMut::from("Content-Length: 5\r\n\r\nhel");
        let mut codec = LspCodec;
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_returns_full_message() {
        let mut buf = BytesMut::from("Content-Length: 7\r\n\r\n{\"a\":1}");
        let mut codec = LspCodec;
        let msg = codec.decode(&mut buf).unwrap();
        assert_eq!(msg.as_deref(), Some("{\"a\":1}"));
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_handles_back_to_back_messages() {
        let mut buf =
            BytesMut::from("Content-Length: 3\r\n\r\n{}\nContent-Length: 7\r\n\r\n{\"b\":2}");
        // First decode pulls "{}\n", second pulls "{\"b\":2}".
        let mut codec = LspCodec;
        let a = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(a, "{}\n");
        let b = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(b, "{\"b\":2}");
    }

    #[test]
    fn decode_rejects_missing_content_length() {
        let mut buf = BytesMut::from("X-Other: foo\r\n\r\nbody");
        let mut codec = LspCodec;
        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, CodecError::MissingLength));
    }

    #[test]
    fn encode_writes_header_and_body() {
        let mut buf = BytesMut::new();
        let mut codec = LspCodec;
        Encoder::encode(&mut codec, "{\"x\":1}", &mut buf).unwrap();
        assert_eq!(&buf[..], b"Content-Length: 7\r\n\r\n{\"x\":1}");
    }
}
