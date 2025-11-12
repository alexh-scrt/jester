use std::{collections::HashSet, net::SocketAddr, str::FromStr, time::Duration};

use anyhow::{bail, Context, Result};
use http::Uri;
use serde::{Deserialize, Serialize};

/// Root configuration structure deserialized from TOML/JSON/YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub admin: Option<Admin>,
    pub listeners: Vec<Listener>,
    pub routes: Vec<Route>,
    pub plugins: Option<Plugins>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Admin {
    pub listen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Listener {
    pub name: String,
    pub bind: String,
    pub tls: Option<Tls>,
    pub alpn: Option<Vec<String>>,
    pub http: Option<HttpTweaks>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tls {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HttpTweaks {
    pub max_header_bytes: Option<u32>,
    pub request_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Route {
    pub name: String,
    pub matchers: Matchers,
    pub filters: Vec<Filter>,
    pub upstream: Upstream,
    #[serde(default)]
    pub response_filters: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Matchers {
    pub hosts: Option<Vec<String>>,
    pub path_prefix: Option<String>,
    pub methods: Option<Vec<String>>,
    pub headers: Option<Vec<HeaderMatch>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Filter {
    #[serde(rename = "builtin")]
    Builtin {
        name: String,
        #[serde(default)]
        config: serde_json::Value,
    },
    #[serde(rename = "wasm")]
    Wasm {
        name: String,
        module: String,
        #[serde(default)]
        config: serde_json::Value,
    },
    #[serde(rename = "inproc")]
    InProc {
        name: String,
        symbol: String,
        #[serde(default)]
        config: serde_json::Value,
    },
}

impl Default for Filter {
    fn default() -> Self {
        Filter::Builtin {
            name: String::new(),
            config: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy")]
pub enum Upstream {
    #[serde(rename = "single")]
    Single { target: String },
    #[serde(rename = "round_robin")]
    RoundRobin { targets: Vec<String> },
    #[serde(rename = "least_latency")]
    LeastLatency { targets: Vec<String> },
    #[serde(rename = "hash")]
    Hash { targets: Vec<String>, key: String },
}

impl Default for Upstream {
    fn default() -> Self {
        Upstream::Single {
            target: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Plugins {
    pub search_paths: Vec<String>,
    pub allow_unsafe_dylib: bool,
}

impl Config {
    /// Validates structural invariants and provides actionable error messages.
    pub fn validate(&self) -> Result<()> {
        if self.listeners.is_empty() {
            bail!("at least one listener is required");
        }
        let mut listener_names = HashSet::new();
        for listener in &self.listeners {
            listener.validate()?;
            if !listener_names.insert(listener.name.clone()) {
                bail!("duplicate listener name `{}`", listener.name);
            }
        }

        if self.routes.is_empty() {
            bail!("at least one route is required");
        }
        let mut route_names = HashSet::new();
        for route in &self.routes {
            route.validate()?;
            if !route_names.insert(route.name.clone()) {
                bail!("duplicate route name `{}`", route.name);
            }
        }
        Ok(())
    }

    /// Returns parsed listeners with ready-to-bind socket addresses.
    pub fn resolved_listeners(&self) -> Result<Vec<ResolvedListener>> {
        self.listeners
            .iter()
            .map(ResolvedListener::try_from)
            .collect()
    }
}

/// Runtime representation of a listener with parsed socket/tls config.
#[derive(Debug, Clone)]
pub struct ResolvedListener {
    pub name: String,
    pub addr: SocketAddr,
    pub tls: Tls,
    pub alpn: Vec<String>,
}

impl TryFrom<&Listener> for ResolvedListener {
    type Error = anyhow::Error;

    fn try_from(listener: &Listener) -> Result<Self> {
        let addr = listener.parse_bind_addr()?;
        let tls = listener
            .tls
            .clone()
            .context("TLS configuration is required for every listener in v0.0.1")?;
        let alpn = listener
            .alpn
            .clone()
            .unwrap_or_else(|| vec!["h2".into(), "http/1.1".into()]);
        Ok(Self {
            name: listener.name.clone(),
            addr,
            tls,
            alpn,
        })
    }
}

impl Listener {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("listener name must not be empty");
        }
        self.parse_bind_addr()
            .with_context(|| format!("invalid bind address for listener `{}`", self.name))?;
        if let Some(tls) = &self.tls {
            tls.validate()?;
        } else {
            bail!("listener `{}` must specify tls.cert and tls.key", self.name);
        }
        Ok(())
    }

    pub fn parse_bind_addr(&self) -> Result<SocketAddr> {
        if self.bind.starts_with(':') {
            let addr = format!("0.0.0.0{}", self.bind);
            Ok(SocketAddr::from_str(&addr)?)
        } else {
            Ok(SocketAddr::from_str(&self.bind)?)
        }
    }
}

impl Tls {
    pub fn validate(&self) -> Result<()> {
        if self.cert.trim().is_empty() || self.key.trim().is_empty() {
            bail!("tls cert and key paths must be provided");
        }
        Ok(())
    }
}

impl Route {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("route name must not be empty");
        }
        if self
            .matchers
            .hosts
            .as_ref()
            .map_or(true, |hosts| hosts.is_empty())
        {
            bail!(
                "route `{}` must declare at least one host matcher",
                self.name
            );
        }
        self.upstream.validate()?;
        Ok(())
    }

    pub fn request_timeout(&self) -> Option<Duration> {
        self.filters.iter().find_map(|filter| match filter {
            Filter::Builtin { name, config } if name == "timeout" => config
                .get("request_secs")?
                .as_u64()
                .map(Duration::from_secs),
            _ => None,
        })
    }
}

impl Upstream {
    pub fn validate(&self) -> Result<()> {
        match self {
            Upstream::Single { target } => {
                Uri::from_str(target)
                    .with_context(|| format!("invalid upstream target `{target}`"))?;
                Ok(())
            }
            Upstream::RoundRobin { .. } | Upstream::LeastLatency { .. } | Upstream::Hash { .. } => {
                bail!("upstream strategy `{:?}` is not supported in v0.0.1", self)
            }
        }
    }

    pub fn single_target(&self) -> Option<&str> {
        match self {
            Upstream::Single { target } => Some(target.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_bind_shorthand_defaults_to_all_interfaces() {
        let listener = Listener {
            name: "test".into(),
            bind: ":8080".into(),
            tls: Some(Tls {
                cert: "cert".into(),
                key: "key".into(),
            }),
            alpn: None,
            http: None,
        };
        assert_eq!(
            listener.parse_bind_addr().unwrap(),
            SocketAddr::from_str("0.0.0.0:8080").unwrap()
        );
    }

    #[test]
    fn route_timeout_parses_builtin_filter() {
        let mut route = Route::default();
        route.name = "test".into();
        route.matchers.hosts = Some(vec!["example.com".into()]);
        route.upstream = Upstream::Single {
            target: "http://127.0.0.1:8080".into(),
        };
        route.filters.push(Filter::Builtin {
            name: "timeout".into(),
            config: serde_json::json!({ "request_secs": 5 }),
        });
        assert_eq!(route.request_timeout(), Some(Duration::from_secs(5)));
    }
}
