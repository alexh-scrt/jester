use std::{net::IpAddr, str::FromStr, time::Duration};

use anyhow::{Context, Result};
use http::{header::HeaderName, HeaderMap, Method, Request, Uri};

use crate::config::{HeaderMatch, Matchers, Route, Upstream};

#[derive(Clone)]
pub struct Router {
    routes: Vec<RouteHandle>,
}

impl Router {
    pub fn build(routes: &[Route]) -> Result<Self> {
        let handles = routes
            .iter()
            .map(RouteHandle::try_from)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { routes: handles })
    }

    pub fn select<B>(&self, req: &Request<B>, host: &str) -> Option<&RouteHandle> {
        let path = req.uri().path();
        let method = req.method();
        let headers = req.headers();
        self.routes
            .iter()
            .find(|route| route.matchers.matches(host, path, method, headers))
    }
}

#[derive(Clone)]
pub struct RouteHandle {
    pub name: String,
    matchers: RouteMatchers,
    pub upstream: UpstreamEndpoint,
    pub timeout: Option<Duration>,
}

impl RouteHandle {
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }
}

impl TryFrom<&Route> for RouteHandle {
    type Error = anyhow::Error;

    fn try_from(route: &Route) -> Result<Self> {
        Ok(Self {
            name: route.name.clone(),
            matchers: RouteMatchers::try_from(&route.matchers)?,
            upstream: UpstreamEndpoint::try_from(&route.upstream)?,
            timeout: route.request_timeout(),
        })
    }
}

#[derive(Clone)]
pub struct UpstreamEndpoint {
    pub uri: Uri,
}

impl TryFrom<&Upstream> for UpstreamEndpoint {
    type Error = anyhow::Error;

    fn try_from(value: &Upstream) -> Result<Self> {
        let target = value
            .single_target()
            .context("v0.0.1 only supports a single upstream target per route")?;
        let uri = Uri::from_str(target)?;
        Ok(Self { uri })
    }
}

#[derive(Clone)]
struct RouteMatchers {
    hosts: Vec<HostMatcher>,
    path_prefix: Option<String>,
    methods: Option<Vec<Method>>,
    headers: Vec<HeaderPredicate>,
}

impl RouteMatchers {
    fn matches(&self, host: &str, path: &str, method: &Method, headers: &HeaderMap) -> bool {
        if !self.hosts.is_empty() && !self.hosts.iter().any(|matcher| matcher.matches(host)) {
            return false;
        }

        if let Some(prefix) = &self.path_prefix {
            if !path.starts_with(prefix) {
                return false;
            }
        }

        if let Some(methods) = &self.methods {
            if !methods.iter().any(|allowed| allowed == method) {
                return false;
            }
        }

        for predicate in &self.headers {
            if !predicate.matches(headers) {
                return false;
            }
        }

        true
    }
}

impl TryFrom<&Matchers> for RouteMatchers {
    type Error = anyhow::Error;

    fn try_from(matchers: &Matchers) -> Result<Self> {
        let hosts = matchers
            .hosts
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|pattern| HostMatcher::new(pattern.as_str()))
            .collect::<Result<Vec<_>>>()?;

        let methods = matchers.methods.as_ref().map(|items| {
            items
                .iter()
                .filter_map(|m| Method::from_bytes(m.as_bytes()).ok())
                .collect::<Vec<_>>()
        });

        let headers = matchers
            .headers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|h| HeaderPredicate::try_from(&h).ok())
            .collect();

        Ok(Self {
            hosts,
            path_prefix: matchers.path_prefix.clone(),
            methods,
            headers,
        })
    }
}

#[derive(Clone)]
enum HostMatcher {
    Any,
    Exact(String),
    Wildcard(String),
    Ip(IpAddr),
}

impl HostMatcher {
    fn new(pattern: &str) -> Result<Self> {
        if pattern == "*" {
            return Ok(Self::Any);
        }
        if let Ok(ip) = pattern.parse::<IpAddr>() {
            return Ok(Self::Ip(ip));
        }
        if let Some(stripped) = pattern.strip_prefix("*.") {
            return Ok(Self::Wildcard(stripped.to_string()));
        }
        Ok(Self::Exact(pattern.to_string()))
    }

    fn matches(&self, host: &str) -> bool {
        match self {
            HostMatcher::Any => true,
            HostMatcher::Exact(value) => host.eq_ignore_ascii_case(value),
            HostMatcher::Wildcard(suffix) => host
                .to_ascii_lowercase()
                .ends_with(&suffix.to_ascii_lowercase()),
            HostMatcher::Ip(expected) => host
                .parse::<IpAddr>()
                .map(|ip| &ip == expected)
                .unwrap_or(false),
        }
    }
}

#[derive(Clone)]
struct HeaderPredicate {
    name: HeaderName,
    value: String,
}

impl HeaderPredicate {
    fn matches(&self, headers: &HeaderMap) -> bool {
        headers
            .get(&self.name)
            .and_then(|value| value.to_str().ok())
            .map(|actual| actual == self.value)
            .unwrap_or(false)
    }
}

impl TryFrom<&HeaderMatch> for HeaderPredicate {
    type Error = anyhow::Error;

    fn try_from(value: &HeaderMatch) -> Result<Self> {
        Ok(Self {
            name: HeaderName::from_str(&value.name)?,
            value: value.value.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_matcher(hosts: Vec<&str>, host: &str, path: &str) -> bool {
        let matchers = Matchers {
            hosts: Some(hosts.into_iter().map(String::from).collect()),
            path_prefix: Some("/api".into()),
            methods: None,
            headers: None,
        };
        let rm = RouteMatchers::try_from(&matchers).unwrap();
        let request = Request::builder().uri("/api/test").body(()).unwrap();
        rm.matches(
            host,
            request.uri().path(),
            request.method(),
            request.headers(),
        )
    }

    #[test]
    fn wildcard_hosts_match_suffix() {
        assert!(test_matcher(vec!["*.svc.local"], "foo.svc.local", "/api"));
        assert!(!test_matcher(vec!["*.svc.local"], "foo.svc", "/api"));
    }

    #[test]
    fn exact_hosts_match_case_insensitive() {
        assert!(test_matcher(vec!["Example.com"], "example.com", "/api"));
    }
}
