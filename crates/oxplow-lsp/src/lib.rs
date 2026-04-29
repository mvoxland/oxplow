//! LSP proxy crate.
//!
//! Spawns a language-server subprocess (e.g.
//! `typescript-language-server --stdio`), frames JSON-RPC messages
//! according to the LSP base protocol (Content-Length-prefixed
//! application/vscode-jsonrpc), and exposes:
//!   - `request(method, params)` — JSON-RPC request, awaits response
//!   - `notify(method, params)` — fire-and-forget notification
//!   - `events()` — stream of server-originated messages (notifications
//!     and server→client requests) the caller must route
//!
//! Higher-level session management (which language ID maps to which
//! command, multiplexing browser clients) lives in `oxplow-app`. This
//! crate is intentionally a transport layer, nothing more.

mod codec;
mod proxy;

pub use codec::CodecError;
pub use proxy::{LspError, LspProxy, ServerEvent, SpawnConfig};
