//! nib-core — the document / geometry / operation engine.
//!
//! This crate is the single source of truth for nib's editing logic. It compiles to:
//!   * **WASM** (`wasm-pack build core --target web`) — the browser editor drives it
//!     locally for 60fps, offline-capable editing.
//!   * **native** (`cargo build` / `cargo test`, and later the rust-axum backend) — the
//!     authority for persistence + realtime sync + the MCP tool surface.
//!
//! Phase A1 is a deliberately trivial scaffold: it proves the Rust → WASM → Svelte pipeline
//! end to end (construct an `Editor`, round-trip a value across the boundary) before the
//! model, op vocabulary, and geometry are ported in (A2/A3).

use wasm_bindgen::prelude::*;

pub mod model;
pub mod snap;

/// The nib editing engine. It **owns** the document + history + selection across calls;
/// the JS side holds one `Editor` handle and drives it with ops and queries. In A1 it holds
/// only its build version — enough to confirm the boundary works.
#[wasm_bindgen]
pub struct Editor {
    build: String,
}

#[wasm_bindgen]
impl Editor {
    /// Construct the engine. Exposed to JS as `new Editor()`.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Editor {
        Editor {
            build: core_version(),
        }
    }

    /// Round-trip a string across the WASM boundary — the A1 smoke test that the core is
    /// live and callable from the browser.
    pub fn echo(&self, msg: &str) -> String {
        format!("nib-core {} · {msg}", self.build)
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// The core crate version, surfaced in the UI to confirm the WASM engine is loaded.
#[wasm_bindgen]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo() {
        assert_eq!(core_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn echo_roundtrips_the_message() {
        let ed = Editor::new();
        let out = ed.echo("hello");
        assert!(
            out.contains("hello"),
            "echo should include the message: {out}"
        );
        assert!(out.contains("nib-core"), "echo should tag the core: {out}");
    }
}
