use serde::{Deserialize, Serialize};
use serde_json::Value;

/// On-disk JSON manifest located next to each plugin artifact.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub module: Option<String>,
    pub capabilities: Vec<String>,
    pub config_schema: Option<Value>,
}

impl PluginManifest {
    pub fn requires_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
}
