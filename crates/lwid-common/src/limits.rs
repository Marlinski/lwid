//! Shared size limits and TTL parsing for the lookwhatidid platform.
//!
//! These constants are referenced by both the server (enforcement) and the CLI
//! (pre-flight validation), ensuring consistent behaviour across the stack.

use chrono::{DateTime, TimeDelta, Utc};

/// Maximum size of a single blob upload, in bytes (10 MB).
pub const MAX_BLOB_SIZE: usize = 10 * 1024 * 1024;

/// Maximum total size of all files in a single project, in bytes (10 MB).
pub const MAX_PROJECT_SIZE: usize = 10 * 1024 * 1024;

/// Default server URL.
pub const DEFAULT_SERVER: &str = "https://lookwhatidid.ovh";

/// Default TTL for new projects.
pub const DEFAULT_TTL: &str = "7d";

/// Valid TTL choices for display.
pub const TTL_CHOICES: &[&str] = &["1h", "1d", "7d", "30d", "never"];

/// Parse a TTL string into an optional expiry timestamp relative to `now`.
///
/// Accepted values: `"1h"`, `"1d"`, `"7d"`, `"30d"`, `"never"`.
/// Returns `None` for `"never"` (no expiry), `Some(expiry)` otherwise.
/// Returns `Err` for unrecognized strings.
pub fn parse_ttl(ttl: &str, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, String> {
    let delta = match ttl {
        "1h" => TimeDelta::hours(1),
        "1d" => TimeDelta::days(1),
        "7d" => TimeDelta::days(7),
        "30d" => TimeDelta::days(30),
        "never" => return Ok(None),
        _ => {
            return Err(format!(
                "invalid TTL '{ttl}': expected one of {}",
                TTL_CHOICES.join(", ")
            ))
        }
    };
    Ok(Some(now + delta))
}

/// Format a byte count as a human-readable string (e.g. "1.5 MB", "320 KB").
pub fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_correctly() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(1_048_576), "1.0 MB");
        assert_eq!(human_bytes(10 * 1_048_576), "10.0 MB");
    }

    #[test]
    fn parse_ttl_valid() {
        let now = Utc::now();
        assert!(parse_ttl("1h", now).unwrap().is_some());
        assert!(parse_ttl("1d", now).unwrap().is_some());
        assert!(parse_ttl("7d", now).unwrap().is_some());
        assert!(parse_ttl("30d", now).unwrap().is_some());
        assert!(parse_ttl("never", now).unwrap().is_none());
    }

    #[test]
    fn parse_ttl_invalid() {
        assert!(parse_ttl("2h", Utc::now()).is_err());
        assert!(parse_ttl("", Utc::now()).is_err());
    }

    #[test]
    fn parse_ttl_duration() {
        let now = Utc::now();
        let exp = parse_ttl("1d", now).unwrap().unwrap();
        let diff = exp - now;
        assert!((diff.num_seconds() - 86400).abs() < 2);
    }
}
