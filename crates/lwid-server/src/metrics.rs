//! Prometheus exporter, served on its own listener.
//!
//! `GET /metrics` on a second port (see [`crate::config::MetricsConfig`]),
//! deliberately *not* on the public router: these numbers describe the whole
//! deployment — user counts, storage totals — and nothing about them belongs
//! on an origin that serves untrusted project content. Binding to loopback by
//! default means it is unreachable from outside the pod unless an operator
//! opts in.
//!
//! # What it costs
//!
//! A scrape is answered from a [`CACHE_TTL`]-second memoised snapshot, so a
//! tight scrape interval cannot amplify into a storm of S3 calls. Behind that
//! cache, one refresh costs:
//!
//! - four grouped `COUNT` queries against SQLite (free; skipped when auth is
//!   off and there is no database at all);
//! - one `ProjectStore::list()`;
//! - one `BlobStore::usage()` — a single paginated LIST;
//! - and, when [`MetricsConfig::project_detail`] is on, one `get()` per
//!   project, which is the only part that grows with the deployment.
//!
//! That last group is why `project_detail` exists as a switch and why it
//! stops fanning out past [`MetricsConfig::project_detail_limit`]: the
//! per-project breakdown is the most interesting part of this exporter right
//! up until the point where producing it is the most expensive thing the
//! server does, and that crossover should not require a code change to
//! survive.
//!
//! HTTP request rate/latency/status are intentionally absent — the ingress
//! already exports those per host, and duplicating them in-process adds
//! nothing but disagreement between two sources.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use tokio::sync::Mutex;
use tracing::warn;

use crate::api::AppState;

/// How long a rendered snapshot is served before it is recomputed.
pub const CACHE_TTL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Exposition format
// ---------------------------------------------------------------------------

/// Accumulates metrics in the Prometheus text exposition format.
///
/// Hand-rolled rather than pulled from a crate: everything here is a gauge
/// written once per scrape, so the entire surface needed is "a name, some
/// labels, a number" — and that keeps the escaping rules visible and tested
/// instead of implied.
#[derive(Default)]
pub struct Exposition {
    out: String,
}

impl Exposition {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a gauge family's `# HELP` / `# TYPE` header.
    fn header(&mut self, name: &str, help: &str) {
        let _ = writeln!(self.out, "# HELP {name} {help}");
        let _ = writeln!(self.out, "# TYPE {name} gauge");
    }

    /// A single unlabelled gauge.
    pub fn gauge(&mut self, name: &str, help: &str, value: impl Into<f64>) {
        self.header(name, help);
        let _ = writeln!(self.out, "{name} {}", fmt_value(value.into()));
    }

    /// A gauge family with one sample per label set.
    ///
    /// Emits the `# HELP`/`# TYPE` header even when `samples` is empty, so a
    /// family that currently has no series is still visibly present (and
    /// typed) rather than indistinguishable from one that was never exported.
    pub fn gauge_vec(
        &mut self,
        name: &str,
        help: &str,
        label_names: &[&str],
        samples: &[(Vec<String>, f64)],
    ) {
        self.header(name, help);
        for (label_values, value) in samples {
            let labels = label_names
                .iter()
                .zip(label_values)
                .map(|(k, v)| format!("{k}=\"{}\"", escape_label(v)))
                .collect::<Vec<_>>()
                .join(",");
            let _ = writeln!(self.out, "{name}{{{labels}}} {}", fmt_value(*value));
        }
    }

    pub fn finish(self) -> String {
        self.out
    }
}

/// Escape a label value per the exposition format: backslash, double quote
/// and newline are the only characters that need it.
fn escape_label(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a value without a trailing `.0` for integers, which keeps the
/// output readable and matches what every other exporter emits.
fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// Gather every metric and render the exposition body.
async fn collect(state: &AppState) -> String {
    let started = Instant::now();
    let mut e = Exposition::new();

    e.gauge_vec(
        "lwid_build_info",
        "Build information; always 1, carries the version as a label.",
        &["version"],
        &[(vec![env!("LWID_VERSION").to_owned()], 1.0)],
    );

    collect_db(state, &mut e).await;
    collect_projects(state, &mut e).await;
    collect_storage(state, &mut e).await;

    e.gauge(
        "lwid_metrics_scrape_duration_seconds",
        "Time the last uncached metrics refresh took.",
        started.elapsed().as_secs_f64(),
    );

    e.finish()
}

/// SQLite-backed metrics. Silently absent when auth is disabled — there is no
/// database then, and exporting zeros would assert something false.
async fn collect_db(state: &AppState, e: &mut Exposition) {
    let Some(pool) = state.db.as_deref() else {
        return;
    };

    match crate::db::count_users_by_provider_tier(pool).await {
        Ok(rows) => e.gauge_vec(
            "lwid_users_total",
            "Registered users, by identity provider and tier.",
            &["provider", "tier"],
            &rows
                .into_iter()
                .map(|(p, t, n)| (vec![p, t], n as f64))
                .collect::<Vec<_>>(),
        ),
        Err(err) => warn!(error = %err, "metrics: users query failed"),
    }

    match crate::db::count_active_sessions_by_kind(pool).await {
        Ok(rows) => e.gauge_vec(
            "lwid_sessions_active",
            "Sessions that have not yet expired, by kind.",
            &["kind"],
            &rows
                .into_iter()
                .map(|(k, n)| (vec![k], n as f64))
                .collect::<Vec<_>>(),
        ),
        Err(err) => warn!(error = %err, "metrics: sessions query failed"),
    }

    match crate::db::count_signed_in_users(pool).await {
        Ok(n) => e.gauge(
            "lwid_signed_in_users",
            "Distinct users holding at least one unexpired session.",
            n as f64,
        ),
        Err(err) => warn!(error = %err, "metrics: signed-in users query failed"),
    }

    match crate::db::count_project_owners(pool).await {
        Ok(n) => e.gauge(
            "lwid_project_owners_total",
            "Projects claimed by a signed-in user; the rest are anonymous.",
            n as f64,
        ),
        Err(err) => warn!(error = %err, "metrics: project owners query failed"),
    }
}

/// Project counts. The totals are one `list()`; the breakdown costs a `get()`
/// per project and is gated by config and by a size ceiling.
async fn collect_projects(state: &AppState, e: &mut Exposition) {
    let ids = match state.projects.list().await {
        Ok(ids) => ids,
        Err(err) => {
            warn!(error = %err, "metrics: project list failed");
            return;
        }
    };

    e.gauge(
        "lwid_projects_total",
        "Projects known to the server, live and expired alike.",
        ids.len() as f64,
    );

    let cfg = &state.config.metrics;
    let within_limit = ids.len() <= cfg.project_detail_limit;
    let detailed = cfg.project_detail && within_limit;

    e.gauge(
        "lwid_project_detail_enabled",
        "1 when the per-project breakdown below is being collected; 0 when it \
         is switched off or the project count exceeds project_detail_limit.",
        if detailed { 1.0 } else { 0.0 },
    );

    if !detailed {
        if cfg.project_detail && !within_limit {
            warn!(
                projects = ids.len(),
                limit = cfg.project_detail_limit,
                "metrics: project count over limit, skipping per-project breakdown",
            );
        }
        return;
    }

    let now = Utc::now();
    let (mut with_content, mut expiring, mut expired, mut blob_refs) = (0u64, 0u64, 0u64, 0u64);
    let mut by_version: std::collections::BTreeMap<String, u64> = Default::default();

    for id in &ids {
        let project = match state.projects.get(id).await {
            Ok(p) => p,
            // A project can be reaped between the list and the get; that is
            // ordinary, not an error worth failing the whole scrape over.
            Err(_) => continue,
        };

        if project.root_cid.is_some() {
            with_content += 1;
        }
        match project.expires_at {
            Some(exp) if exp <= now => expired += 1,
            Some(_) => expiring += 1,
            None => {}
        }
        blob_refs += project.blob_cids.len() as u64;
        *by_version
            .entry(project.created_with.unwrap_or_else(|| "unknown".to_owned()))
            .or_default() += 1;
    }

    e.gauge(
        "lwid_projects_with_content",
        "Projects that have published at least one version.",
        with_content as f64,
    );
    e.gauge(
        "lwid_projects_expiring",
        "Projects with a future expiry deadline.",
        expiring as f64,
    );
    e.gauge(
        "lwid_projects_expired",
        "Projects past their deadline, awaiting the reaper's next sweep.",
        expired as f64,
    );
    e.gauge(
        "lwid_project_blob_refs",
        "Total blob references held across all projects; counts shared blobs \
         once per referencing project, so it exceeds lwid_blobs_total.",
        blob_refs as f64,
    );
    e.gauge_vec(
        "lwid_projects_by_version",
        "Projects by the client version that created them.",
        &["created_with"],
        &by_version
            .into_iter()
            .map(|(v, n)| (vec![v], n as f64))
            .collect::<Vec<_>>(),
    );
}

/// Blob store size — one enumeration of the store.
async fn collect_storage(state: &AppState, e: &mut Exposition) {
    match state.blobs.usage().await {
        Ok(usage) => {
            e.gauge(
                "lwid_blobs_total",
                "Blobs in the content-addressed store.",
                usage.objects as f64,
            );
            e.gauge(
                "lwid_storage_bytes",
                "Bytes occupied by the content-addressed blob store.",
                usage.bytes as f64,
            );
        }
        Err(err) => warn!(error = %err, "metrics: blob store usage failed"),
    }
}

// ---------------------------------------------------------------------------
// Cache + router
// ---------------------------------------------------------------------------

/// The memoised snapshot shared by every scrape.
#[derive(Default)]
struct Cache {
    rendered: Option<(Instant, String)>,
}

#[derive(Clone)]
pub struct MetricsState {
    app: AppState,
    cache: Arc<Mutex<Cache>>,
}

/// Build the metrics router. Mounted on its own listener, never on the
/// public one.
pub fn router(app: AppState) -> Router {
    Router::new()
        .route("/metrics", get(handler))
        .route("/health", get(|| async { "ok\n" }))
        .with_state(MetricsState {
            app,
            cache: Arc::new(Mutex::new(Cache::default())),
        })
}

async fn handler(State(state): State<MetricsState>) -> Response {
    // The lock is held across the refresh on purpose: concurrent scrapes wait
    // for the one in flight and then share its result, rather than each
    // starting its own fan-out.
    let mut cache = state.cache.lock().await;

    if let Some((at, body)) = cache.rendered.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return text(body.clone());
    }

    let body = collect(&state.app).await;
    cache.rendered = Some((Instant::now(), body.clone()));
    text(body)
}

fn text(body: String) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_renders_help_type_and_value() {
        let mut e = Exposition::new();
        e.gauge("lwid_thing", "A thing.", 42.0);
        assert_eq!(
            e.finish(),
            "# HELP lwid_thing A thing.\n# TYPE lwid_thing gauge\nlwid_thing 42\n",
        );
    }

    #[test]
    fn integers_render_without_a_decimal_point() {
        assert_eq!(fmt_value(0.0), "0");
        assert_eq!(fmt_value(80_190_035.0), "80190035");
        assert_eq!(fmt_value(1.5), "1.5");
    }

    #[test]
    fn labels_are_escaped() {
        // A created_with string is client-supplied, so it reaches the
        // exposition as a label value and must not be able to break out of it.
        assert_eq!(escape_label(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_label(r"a\b"), r"a\\b");
        assert_eq!(escape_label("a\nb"), r"a\nb");
    }

    #[test]
    fn gauge_vec_emits_one_line_per_label_set() {
        let mut e = Exposition::new();
        e.gauge_vec(
            "lwid_users_total",
            "Users.",
            &["provider", "tier"],
            &[
                (vec!["github".into(), "free".into()], 3.0),
                (vec!["google".into(), "pro".into()], 1.0),
            ],
        );
        let out = e.finish();
        assert!(out.contains("lwid_users_total{provider=\"github\",tier=\"free\"} 3\n"));
        assert!(out.contains("lwid_users_total{provider=\"google\",tier=\"pro\"} 1\n"));
    }

    #[test]
    fn empty_family_still_declares_help_and_type() {
        let mut e = Exposition::new();
        e.gauge_vec("lwid_users_total", "Users.", &["provider"], &[]);
        let out = e.finish();
        assert!(out.contains("# TYPE lwid_users_total gauge"));
        assert_eq!(out.lines().count(), 2);
    }
}
