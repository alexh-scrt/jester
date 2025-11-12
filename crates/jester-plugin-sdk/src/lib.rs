pub mod manifest;

pub use manifest::PluginManifest;

use serde_json::Value;

/// Trait implemented by WASM or native plugins compiled outside the core workspace.
pub trait Plugin {
    fn name(&self) -> &'static str;
    fn version(&self) -> semver::Version;
    fn init(&mut self, config: Value) -> anyhow::Result<()>;
    fn capabilities(&self) -> &'static [&'static str];
}

/// Reference WIT interface exposed by the host runtime.
pub const HTTP_WIT: &str = include_str!("../wit/http.wit");
