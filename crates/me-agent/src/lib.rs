//! Codex-facing processes. No database keys cross the bridge.
mod bridge;
pub mod codex;
mod protocol;
pub use bridge::{BridgeServer, bridge_call};
pub use protocol::{run_mcp, tools_list};

use std::path::PathBuf;
pub fn default_vault_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ME_VAULT_DIR") {
        let path = PathBuf::from(path);
        return path.is_absolute().then_some(path);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
    };
    Some(base.join("ME/vault"))
}

pub mod typesafe;
