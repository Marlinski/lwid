//! Canonical-host redirect.
//!
//! The service answers on more than one hostname, but only one of them is the
//! name people should see. Requests arriving on a legacy hostname are answered
//! with a permanent redirect to the canonical one.
//!
//! This lives in the application rather than in the ingress because the
//! `nginx.ingress.kubernetes.io/permanent-redirect` annotation emits a bare
//! `return 301 <url>;`, which discards the request path — every shared
//! `/p/{id}` link would land on the homepage. The annotation also rejects
//! `$request_uri`, and enabling nginx snippets is a cluster-wide setting.
//!
//! The URL fragment needs no handling: it never reaches the server, and
//! browsers re-attach it to the redirect target when that target has no
//! fragment of its own. A `/p/{id}#key` link therefore keeps working.

use axum::extract::{Request, State};
use axum::http::{header, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api::AppState;

/// Strip any `:port` suffix from a `Host` header value.
///
/// IPv6 literals are bracketed (`[::1]:8080`), so only split after the
/// closing bracket.
fn host_without_port(host: &str) -> &str {
    match host.rfind(']') {
        Some(end) => &host[..=end],
        None => host.split(':').next().unwrap_or(host),
    }
}

/// Build the absolute redirect target for a request.
///
/// Returns `None` when the request should be served normally: no canonical
/// host configured, no `Host` header, or a host that is not on the redirect
/// list. Matching is case-insensitive, since `Host` is not case-sensitive.
pub fn redirect_target(
    host_header: Option<&str>,
    uri: &Uri,
    canonical_host: Option<&str>,
    redirect_hosts: &[String],
) -> Option<String> {
    let canonical = canonical_host?;
    if redirect_hosts.is_empty() {
        return None;
    }

    let host = host_without_port(host_header?);
    if host.eq_ignore_ascii_case(canonical) {
        return None;
    }
    if !redirect_hosts
        .iter()
        .any(|h| host_without_port(h).eq_ignore_ascii_case(host))
    {
        return None;
    }

    // path_and_query keeps the query string; it is empty only for
    // authority-form requests (CONNECT), where "/" is the right target.
    let path_and_query = uri.path_and_query().map_or("/", |pq| pq.as_str());
    Some(format!("https://{canonical}{path_and_query}"))
}

/// Middleware answering legacy hostnames with a 301 to the canonical host.
pub async fn canonical_host(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let target = redirect_target(
        host.as_deref(),
        request.uri(),
        state.config.server.canonical_host.as_deref(),
        &state.config.server.redirect_hosts,
    );

    match target {
        Some(url) => (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, url)]).into_response(),
        None => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANON: Option<&str> = Some("lookwhatidid.ovhcloud.tools");

    fn legacy() -> Vec<String> {
        vec!["lookwhatidid.ovh".to_owned()]
    }

    fn uri(s: &str) -> Uri {
        s.parse().expect("test uri")
    }

    #[test]
    fn redirects_a_legacy_host_and_keeps_the_path() {
        assert_eq!(
            redirect_target(Some("lookwhatidid.ovh"), &uri("/p/abc123"), CANON, &legacy()),
            Some("https://lookwhatidid.ovhcloud.tools/p/abc123".to_owned()),
        );
    }

    #[test]
    fn keeps_the_query_string() {
        assert_eq!(
            redirect_target(Some("lookwhatidid.ovh"), &uri("/docs.html?a=1&b=2"), CANON, &legacy()),
            Some("https://lookwhatidid.ovhcloud.tools/docs.html?a=1&b=2".to_owned()),
        );
    }

    #[test]
    fn root_redirects_to_root() {
        assert_eq!(
            redirect_target(Some("lookwhatidid.ovh"), &uri("/"), CANON, &legacy()),
            Some("https://lookwhatidid.ovhcloud.tools/".to_owned()),
        );
    }

    #[test]
    fn ignores_the_port_and_letter_case() {
        for host in ["lookwhatidid.ovh:443", "LookWhatIDid.OVH", "LOOKWHATIDID.OVH:8080"] {
            assert!(
                redirect_target(Some(host), &uri("/x"), CANON, &legacy()).is_some(),
                "expected {host} to redirect",
            );
        }
    }

    #[test]
    fn the_canonical_host_is_served_not_redirected() {
        assert_eq!(
            redirect_target(Some("lookwhatidid.ovhcloud.tools"), &uri("/"), CANON, &legacy()),
            None,
        );
    }

    #[test]
    fn unlisted_hosts_are_served_normally() {
        // Health probes arrive with the pod IP, and in-cluster callers use the
        // service name. Redirecting either would be wrong.
        for host in ["10.2.3.28:8080", "lwid-server.lwid.svc:8080", "localhost:8080"] {
            assert_eq!(
                redirect_target(Some(host), &uri("/"), CANON, &legacy()),
                None,
                "{host} must not be redirected",
            );
        }
    }

    #[test]
    fn does_nothing_without_configuration() {
        assert_eq!(redirect_target(Some("lookwhatidid.ovh"), &uri("/"), None, &legacy()), None);
        assert_eq!(redirect_target(Some("lookwhatidid.ovh"), &uri("/"), CANON, &[]), None);
    }

    #[test]
    fn a_missing_host_header_is_served_normally() {
        assert_eq!(redirect_target(None, &uri("/"), CANON, &legacy()), None);
    }

    #[test]
    fn ipv6_literals_keep_their_brackets() {
        assert_eq!(host_without_port("[::1]:8080"), "[::1]");
        assert_eq!(host_without_port("[::1]"), "[::1]");
        assert_eq!(host_without_port("example.com:80"), "example.com");
    }
}
