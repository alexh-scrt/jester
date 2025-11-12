use std::{net::SocketAddr, sync::Arc, time::Instant};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use http::{header, StatusCode, Uri};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::server::conn::http1;
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioIo},
};
use tokio::{net::TcpListener, sync::watch, task::JoinSet, time::timeout};
use tokio_rustls::{
    rustls::{Certificate, PrivateKey, ServerConfig},
    TlsAcceptor,
};

use crate::{
    config::{Config, ResolvedListener},
    router::{RouteHandle, Router},
};

type ProxyBody = BoxBody<Bytes, hyper::Error>;
type HttpClient = Client<HttpConnector, Incoming>;

/// Primary proxy runtime handle.
pub struct Proxy {
    state: Arc<AppState>,
    listeners: Vec<ListenerRuntime>,
}

struct AppState {
    router: Router,
    client: HttpClient,
}

struct ListenerRuntime {
    name: String,
    addr: SocketAddr,
    acceptor: TlsAcceptor,
}

impl Proxy {
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let router = Router::build(&config.routes)?;
        let listeners = config
            .resolved_listeners()?
            .into_iter()
            .map(ListenerRuntime::try_from)
            .collect::<Result<Vec<_>>>()?;
        let client = build_client();
        let state = Arc::new(AppState { router, client });
        Ok(Self { state, listeners })
    }

    pub async fn run(self) -> Result<()> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut join_set = JoinSet::new();
        for listener in self.listeners {
            let rx = shutdown_rx.clone();
            let state = self.state.clone();
            join_set.spawn(async move { serve_listener(listener, state, rx).await });
        }

        tracing::info!("proxy listeners started; awaiting shutdown signal (Ctrl+C)");
        tokio::signal::ctrl_c()
            .await
            .context("failed to install ctrl-c handler")?;
        tracing::info!("shutdown signal received; draining listeners");
        shutdown_tx.send(true).ok();

        while let Some(result) = join_set.join_next().await {
            if let Err(err) = result {
                tracing::error!(error = %err, "listener task aborted");
            }
        }

        Ok(())
    }
}

fn build_client() -> HttpClient {
    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    Client::builder(TokioExecutor::new()).build(connector)
}

async fn serve_listener(
    listener: ListenerRuntime,
    state: Arc<AppState>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let tcp = TcpListener::bind(listener.addr)
        .await
        .with_context(|| format!("failed to bind listener `{}`", listener.name))?;
    tracing::info!(
        listener = listener.name,
        addr = %listener.addr,
        "listener ready"
    );

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                tracing::info!(listener = listener.name, "listener shutting down");
                break;
            }
            accept = tcp.accept() => {
                let (stream, peer_addr) = accept?;
                let acceptor = listener.acceptor.clone();
                let state = state.clone();
                let listener_name = listener.name.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(acceptor, state, stream, peer_addr, listener_name).await {
                        tracing::warn!(error = %err, "connection closed with error");
                    }
                });
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    acceptor: TlsAcceptor,
    state: Arc<AppState>,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    listener_name: String,
) -> Result<()> {
    let tls = acceptor.accept(stream).await?;
    let service = service_fn(move |req| {
        let state = state.clone();
        async move {
            match handle_request(state, req).await {
                Ok(resp) => Ok::<_, hyper::Error>(resp),
                Err(err) => {
                    tracing::error!(error = %err, "request handling failed");
                    Ok(internal_error())
                }
            }
        }
    });
    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .serve_connection(TokioIo::new(tls), service)
        .with_upgrades()
        .await
        .with_context(|| {
            format!("connection handling failed for listener `{listener_name}` from {peer_addr}")
        })
}

async fn handle_request(
    state: Arc<AppState>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>> {
    let start = Instant::now();
    let host = extract_host(&req);
    let span = tracing::info_span!(
        "request",
        method = %req.method(),
        path = %req.uri().path(),
        host = host.as_deref().unwrap_or_default(),
        route = tracing::field::Empty,
        status = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    );
    let _enter = span.enter();

    let host_ref = host.as_deref().unwrap_or("");
    let route = match state.router.select(&req, host_ref).cloned() {
        Some(route) => route,
        None => {
            span.record("status", StatusCode::NOT_FOUND.as_u16());
            metrics::counter!("jester_requests_total", "outcome" => "miss").increment(1);
            return Ok(not_found());
        }
    };
    span.record("route", &route.name.as_str());

    metrics::counter!("jester_requests_total", "outcome" => "hit").increment(1);
    let response = proxy_to_upstream(state.clone(), req, &route).await;
    let duration = start.elapsed().as_millis() as u64;

    match response {
        Ok(resp) => {
            span.record("status", resp.status().as_u16());
            span.record("duration_ms", duration as i64);
            Ok(resp.map(|body| body.boxed()))
        }
        Err(err) => {
            span.record("status", StatusCode::BAD_GATEWAY.as_u16());
            span.record("duration_ms", duration as i64);
            tracing::error!(error = %err, route = %route.name, "upstream request failed");
            metrics::counter!("jester_requests_total", "outcome" => "error").increment(1);
            Ok(bad_gateway())
        }
    }
}

async fn proxy_to_upstream(
    state: Arc<AppState>,
    mut req: Request<Incoming>,
    route: &RouteHandle,
) -> Result<Response<Incoming>> {
    let upstream_uri = build_upstream_uri(&route.upstream.uri, req.uri())?;
    rewrite_request(&mut req, &route.upstream.uri, upstream_uri.clone());
    let fut = state.client.request(req);
    let response = if let Some(duration) = route.timeout() {
        timeout(duration, fut)
            .await
            .context("request timed out")??
    } else {
        fut.await?
    };
    Ok(response)
}

fn build_upstream_uri(base: &Uri, incoming: &Uri) -> Result<Uri> {
    let mut parts = base.clone().into_parts();
    parts.path_and_query = incoming.path_and_query().cloned();
    if parts.path_and_query.is_none() {
        parts.path_and_query = Some("/".parse()?);
    }
    Uri::from_parts(parts).context("failed to construct upstream uri")
}

fn rewrite_request<B>(req: &mut Request<B>, base: &Uri, target: Uri) {
    *req.uri_mut() = target;
    clean_hop_by_hop(req.headers_mut());
    if let Some(authority) = base.authority() {
        req.headers_mut().insert(
            header::HOST,
            header::HeaderValue::from_str(authority.as_str()).unwrap(),
        );
    }
    req.headers_mut().insert(
        "x-forwarded-proto",
        header::HeaderValue::from_static("https"),
    );
}

fn clean_hop_by_hop(headers: &mut http::HeaderMap) {
    const HOP_HEADERS: [&str; 6] = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "upgrade",
    ];
    for name in HOP_HEADERS {
        headers.remove(name);
    }
}

fn extract_host<B>(req: &Request<B>) -> Option<String> {
    req.uri().host().map(|host| host.to_string()).or_else(|| {
        req.headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok().map(|s| s.to_string()))
    })
}

fn not_found() -> Response<ProxyBody> {
    response_with(StatusCode::NOT_FOUND, "no matching route")
}

fn bad_gateway() -> Response<ProxyBody> {
    response_with(StatusCode::BAD_GATEWAY, "upstream error")
}

fn internal_error() -> Response<ProxyBody> {
    response_with(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn response_with(status: StatusCode, msg: &'static str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from_static(msg.as_bytes()))
        .map_err(|never| match never {})
        .boxed();
    Response::builder().status(status).body(body).unwrap()
}

impl TryFrom<ResolvedListener> for ListenerRuntime {
    type Error = anyhow::Error;

    fn try_from(value: ResolvedListener) -> Result<Self> {
        let server_config = build_tls_config(&value)?;
        Ok(Self {
            name: value.name,
            addr: value.addr,
            acceptor: TlsAcceptor::from(Arc::new(server_config)),
        })
    }
}

fn build_tls_config(listener: &ResolvedListener) -> Result<ServerConfig> {
    let certs = load_certs(&listener.tls.cert)?;
    let key = load_private_key(&listener.tls.key)?;
    let mut config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("invalid certificate/key pair")?;
    config.alpn_protocols = listener
        .alpn
        .iter()
        .map(|proto| proto.as_bytes().to_vec())
        .collect();
    Ok(config)
}

fn load_certs(path: &str) -> Result<Vec<Certificate>> {
    let data = std::fs::read(path).with_context(|| format!("failed to read cert {path}"))?;
    let mut reader = std::io::Cursor::new(data);
    let raw =
        rustls_pemfile::certs(&mut reader).map_err(|_| anyhow!("invalid certificate data"))?;
    Ok(raw.into_iter().map(Certificate).collect())
}

fn load_private_key(path: &str) -> Result<PrivateKey> {
    let data = std::fs::read(path).with_context(|| format!("failed to read key {path}"))?;
    let mut reader = std::io::Cursor::new(data);
    while let Some(item) =
        rustls_pemfile::read_one(&mut reader).map_err(|_| anyhow!("invalid key format"))?
    {
        match item {
            rustls_pemfile::Item::PKCS8Key(key) => return Ok(PrivateKey(key)),
            rustls_pemfile::Item::RSAKey(key) => return Ok(PrivateKey(key)),
            rustls_pemfile::Item::ECKey(key) => return Ok(PrivateKey(key)),
            _ => continue,
        }
    }
    anyhow::bail!("no usable private keys found in {path}")
}
