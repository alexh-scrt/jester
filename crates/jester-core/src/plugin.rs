use bytes::Bytes;
use http::{Request, Response};
use serde_json::Value;
use tower::{util::BoxService, Layer};

pub type HttpRequest = Request<Bytes>;
pub type HttpResponse = Response<Bytes>;
pub type JesterService = BoxService<HttpRequest, HttpResponse, anyhow::Error>;
pub type DynLayer = Box<dyn Layer<JesterService, Service = JesterService> + Send + Sync>;

/// Canonical plugin trait implemented by core + external extensions.
pub trait JesterPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn version(&self) -> semver::Version;
    fn layer(&self, cfg: Value) -> anyhow::Result<DynLayer>;
    fn capabilities(&self) -> &'static [&'static str];
}
