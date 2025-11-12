
# 1) Pluggable components: three tiers (pick per risk/perf)

Think “layers at the edge” with strong boundaries. You get safety by default, and raw speed where you control the code.

## Tier A — **In-process Rust layers** (fastest; for 1st-party)

* Implemented as **Tower** `Layer`/`Service` stacks (zero-copy, no FFI, best perf).
* Great for core features: routing, header rewrites, rate limiting, retries, circuit breaking, caching.
* Enabled with Cargo **features** (compile-time plugins) or loaded via **registry** at runtime by type name.

**Why:** Tower is the de-facto standard, interoperates with Hyper/h2/H3, and keeps the data model ergonomic.

**Core trait**

```rust
pub trait JesterPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn version(&self) -> semver::Version;
    fn layer(&self, cfg: serde_json::Value) -> anyhow::Result<tower::Layer<JesterSvc>>;
    fn capabilities(&self) -> &'static [&'static str]; // e.g. ["filter:req", "filter:resp", "metrics"]
}
```

## Tier B — **Sandboxed WebAssembly (WASI)** (safe & hot-swappable; 3rd-party)

* Use **WASI** runtimes (e.g., **wasmtime**) + a tiny, versioned **Jester Plugin ABI** in WIT.
* Plugins declare capabilities; host grants minimal, capability-based access (time, random, limited I/O).
* Hot-reload per route without restarting Jester.
* Excellent for community filters, custom auth, transforms; predictable performance (near-native for simple logic).

**Why:** Strong isolation + easy distribution (a `.wasm` file). Safer than `.so` dylibs.

**Example WIT (sketch)**

```wit
package jester:plugin;

interface http {
  type Headers = list<tuple<string, string>>;
  record Request { method: string, uri: string, headers: Headers, body: list<u8> }
  record Response { status: u16, headers: Headers, body: list<u8> }

  // Synchronous filter hook
  http-filter: func(req: Request) -> Result<Response, Request>;
}
```

## Tier C — **Dynamic libraries (`.so`/`.dylib`)** (unsafe; niche use)

* Only for trusted, performance-critical native code you don’t want to compile into the main binary.
* Use an **ABI-stable** boundary (e.g., `abi_stable` crate) to avoid UB.
* Ship disabled by default; require `--allow-unsafe-dylib` to load.

---

## Extension points (uniform across tiers)

* **Listener**: TCP/TLS provider (e.g., rustls, BORINGSSL later), ALPN (h1/h2; future h3 via quinn)
* **Router**: host/path/method/header matching → `Route`
* **Filters**:

  * Request filters (authN/Z, header mutate, body transform)
  * Upstream selection (LB strategies, consistent hashing)
  * Response filters (compression, cache control, HTML injection, WAF)
* **Observability**: metrics, logs, traces (OpenTelemetry)
* **Storage adapters** (optional): cache backends, KV for rate limiting
* **Control plane** (optional later): admin API, hot reload, health

Each extension point consumes the **same plugin trait / WIT interface**, with the host wiring where each hook is invoked.

---

## Lifecycle & safety

* **Registration**: on boot, Jester scans `plugins/` for `.wasm` and optional `.json` manifests.
* **Manifests**: declare `name`, `version`, `capabilities`, `config_schema` (JSON Schema), and optional `resources` needed.
* **Validation**: before activation, host validates config against `config_schema`.
* **Isolation**:

  * WASM: no filesystem/net by default; allow only declared capabilities (e.g., “http:outbound” for token introspection calls)
  * In-proc: run under **timeouts**, **body size limits**, and **cancellation** via `tower::timeout`, `tower::limit`
* **Hot reload**: SIGHUP or `jester reload`. New routes/filters spin up, old ones drain using connection draining and graceful shutdown.

---

# 2) Configuration: simple first, structured forever

**Goals**

* New user: a 10-line file is enough
* Power user: composable, DRY, typed, lintable, reloadable
* Everything is **self-explanatory**: comments, examples, and a `jester config validate` command

## Format & ergonomics

* **TOML** for primary config (friendly & readable)
* Generate **JSON Schema** from your Rust structs (via `schemars`) and ship `jester.schema.json`
* Support **env var interpolation**: `${ENV:DEFAULT}`
* Allow **includes** for large installs: `include = ["routes/*.toml"]`

## Data model (Rust)

```rust
#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct Config {
    pub admin: Option<Admin>,
    pub listeners: Vec<Listener>,
    pub routes: Vec<Route>,
    pub plugins: Option<Plugins>, // global plugin discovery & policies
}

pub struct Admin { pub listen: String } // e.g., "127.0.0.1:9000"
pub struct Listener {
    pub name: String,
    pub bind: String,                // ":443"
    pub tls: Option<Tls>,
    pub alpn: Option<Vec<String>>,   // ["h2","http/1.1"]
    pub http: Option<HttpTweaks>,    // timeouts, max_header_size, etc.
}
pub struct Tls { pub cert: String, pub key: String }

pub struct Route {
    pub name: String,
    pub matchers: Matchers,          // host/path/method/header
    pub filters: Vec<Filter>,        // ordered chain (pre-upstream)
    pub upstream: Upstream,          // where to send
    pub response_filters: Vec<Filter>// ordered chain (post-upstream)
}

pub struct Matchers {
    pub hosts: Option<Vec<String>>,      // ["example.com", "*.svc.local"]
    pub path_prefix: Option<String>,     // "/api"
    pub methods: Option<Vec<String>>,    // ["GET","POST"]
    pub headers: Option<Vec<HeaderMatch>>
}

pub enum Filter {
    Builtin(BuiltinFilter),
    Wasm(WasmRef),
    InProc(InProcRef),
}
```

## Minimal config (TLS-terminating forwarder)

```toml
# jester.toml
[listeners]
# Shorthand for a single listener
name = "edge443"
bind = ":443"
alpn = ["h2", "http/1.1"]

[listeners.tls]
cert = "/etc/jester/certs/fullchain.pem"
key  = "/etc/jester/certs/privkey.pem"

[[routes]]
name = "app"
[routes.matchers]
hosts = ["example.com"]
path_prefix = "/"

# No filters yet; just forward
[routes.upstream]
strategy = "single"
targets  = ["http://127.0.0.1:8080"]
```

## Slightly bigger: add an auth WASM + rate limit

```toml
[plugins]
# Where to discover third-party plugins
search_paths = ["./plugins"] 
# Default security posture
allow_unsafe_dylib = false

[[routes]]
name = "api"
[routes.matchers]
hosts = ["api.example.com"]
path_prefix = "/v1"

# Pre-upstream filters (order matters)
[[routes.filters]]
type = "wasm"
name = "jwt-auth"
module = "plugins/jwt-auth.wasm"
# config is freeform, validated by plugin’s JSON Schema
config = { jwks_url = "https://auth.example.com/.well-known/jwks.json", aud = "api" }

[[routes.filters]]
type = "builtin"
name = "rate-limit"
config = { policy = "local", rpm = 600, key = "ip" }

[routes.upstream]
strategy = "round_robin"
targets  = ["http://127.0.0.1:8081", "http://127.0.0.1:8082"]

# Post-upstream response filters
[[routes.response_filters]]
type = "builtin"
name = "compression"
config = { min_bytes = 1024 }
```

## Power features (without complexity creep)

* **Profiles**: `--profile=prod` → loads `jester.prod.toml` overlay
* **Config introspection**: `jester config schema > jester.schema.json`
* **Lints**: `jester config lint` (dead routes, shadowed matchers)
* **Dry run**: `jester run --dry` validates and prints the resolved plan

---

# Internals overview (incremental milestones)

## v0.1 “Court Entrance” (MVP)

* **Runtime**: Tokio
* **Proto**: Hyper (HTTP/1.1 + h2), rustls for TLS
* **Routing**: Host + path-prefix
* **Upstream**: Single target
* **Config**: Single TOML file, validate on start
* **Observability**: `tracing` + structured logs

## v0.2 “Mask & Bells”

* **Tower** stack; pluggable **builtin** filters (timeout, header, rate limit)
* **OpenTelemetry** traces, Prometheus metrics
* **Hot reload** on SIGHUP; connection draining

## v0.3 “Wit & WASI”

* WASM plugin runtime (wasmtime)
* WIT ABI for request/response filters
* Plugin manifests + JSON Schema validation
* Per-route plugin chains

## v0.4 “The Court”

* LB strategies: round-robin, least-latency, hash
* Health checks, outlier detection
* Response filters (compression, cache headers)
* Admin API (stats, config preview, live reload)

## v0.5+ “Royal Decrees”

* H3/QUIC listener (quinn) behind feature flag
* ACME/auto-cert
* WAF (rule DSL) and mTLS to upstreams
* Policy engine (CEL or OPA/Rego via WASM)

---

# Security posture (defaults matter)

* **TLS by default** (modern suites via rustls)
* **Request limits**: headers/body sizes, parse timeouts
* **No unbounded buffering** (streaming bodies)
* **Least privilege** for plugins (WASM caps off by default)
* **Reproducible builds**: lockfile + SBOM (CycloneDX) + plugin signature verification (optional)

---

# Developer experience

* `jester new plugin --wasm jwt-auth` → scaffolds WIT + tests
* `jester bench` → local load tests for a route
* `jester tap --route api` → live pretty logs for a route
* `jester diag` → dumps effective config + listener/router graph

---
