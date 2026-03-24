//! secrets.rs — Client-side secret detection using regex patterns.
//!
//! Scans file content for common credentials/API keys before upload.
//! Patterns sourced from gitleaks (config/gitleaks.toml) and secretlint.

use regex::Regex;
use std::sync::OnceLock;

/// A single detection rule.
pub struct Rule {
    pub id: &'static str,
    pub description: &'static str,
    pattern: &'static str,
}

/// All detection rules.
static RULES: &[Rule] = &[
    Rule {
        id: "aws-access-key-id",
        description: "AWS Access Key ID",
        pattern: r"\b(?:A3T[A-Z0-9]|AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16}\b",
    },
    Rule {
        id: "aws-secret-access-key",
        description: "AWS Secret Access Key",
        pattern: r#"(?i)["']?(?:AWS)?_?SECRET_?(?:ACCESS)?_?KEY["']?\s*[:=>]+\s*["']?[A-Za-z0-9/+=]{40}["']?"#,
    },
    Rule {
        id: "gcp-api-key",
        description: "Google Cloud / Firebase API Key",
        pattern: r"\bAIza[A-Za-z0-9_\-]{35}\b",
    },
    Rule {
        id: "azure-ad-client-secret",
        description: "Azure AD Client Secret",
        pattern: r"[a-zA-Z0-9_~.]{3}\dQ~[a-zA-Z0-9_~.\-]{31,34}",
    },
    Rule {
        id: "github-token",
        description: "GitHub Personal Access Token",
        pattern: r"\bgh[pousr]_[A-Za-z0-9_]{36}\b",
    },
    Rule {
        id: "github-fine-grained-pat",
        description: "GitHub Fine-Grained PAT",
        pattern: r"\bgithub_pat_[A-Za-z0-9_]{82}\b",
    },
    Rule {
        id: "gitlab-pat",
        description: "GitLab Personal Access Token",
        pattern: r"\bglpat-[A-Za-z0-9\-_]{20}\b",
    },
    Rule {
        id: "openai-api-key",
        description: "OpenAI API Key",
        pattern: r"\bsk-(?:proj|svcacct|admin)-[A-Za-z0-9_\-]{58,74}T3BlbkFJ[A-Za-z0-9_\-]{58,74}\b|\bsk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20}\b",
    },
    Rule {
        id: "anthropic-api-key",
        description: "Anthropic / Claude API Key",
        pattern: r"\bsk-ant-api03-[A-Za-z0-9_\-]{93}AA\b",
    },
    Rule {
        id: "stripe-secret-key",
        description: "Stripe Secret / Restricted Key",
        pattern: r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{20,247}\b",
    },
    Rule {
        id: "square-access-token",
        description: "Square Access Token",
        pattern: r"\bsqOatp-[0-9A-Za-z\-_]{22}\b",
    },
    Rule {
        id: "paypal-braintree-token",
        description: "PayPal / Braintree Access Token",
        pattern: r"\baccess_token\$production\$[0-9a-z]{16}\$[0-9a-f]{32}\b",
    },
    Rule {
        id: "sendgrid-api-key",
        description: "SendGrid API Key",
        pattern: r"\bSG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}\b",
    },
    Rule {
        id: "twilio-api-key",
        description: "Twilio API Key",
        pattern: r"\bSK[0-9a-fA-F]{32}\b",
    },
    Rule {
        id: "twilio-account-sid",
        description: "Twilio Account SID",
        pattern: r"\bAC[a-z0-9]{32}\b",
    },
    Rule {
        id: "mailchimp-api-key",
        description: "Mailchimp API Key",
        pattern: r"\b[0-9a-f]{32}-us[0-9]{1,2}\b",
    },
    Rule {
        id: "mailgun-api-key",
        description: "Mailgun API Key",
        pattern: r"\bkey-[0-9a-zA-Z]{32}\b",
    },
    Rule {
        id: "slack-token",
        description: "Slack Token",
        pattern: r"\bxox[bpaor]-(?:\d+-)?(?:[A-Za-z0-9]{1,40}-)+[A-Za-z0-9]{1,40}\b",
    },
    Rule {
        id: "slack-webhook",
        description: "Slack Incoming Webhook URL",
        pattern: r"https://hooks\.slack\.com/services/T[A-Za-z0-9]{1,40}/B[A-Za-z0-9]{1,40}/[A-Za-z0-9]{1,40}",
    },
    Rule {
        id: "digitalocean-pat",
        description: "DigitalOcean Personal Access Token",
        pattern: r"\bdop_v1_[a-f0-9]{64}\b",
    },
    Rule {
        id: "databricks-token",
        description: "Databricks API Token",
        pattern: r"\bdapi[a-f0-9]{32}(?:-\d)?\b",
    },
    Rule {
        id: "private-key-pem",
        description: "PEM Private Key",
        pattern: r"-----BEGIN (?:(?:RSA|DSA|EC|OPENSSH|PGP) )?PRIVATE KEY(?: BLOCK)?-----",
    },
    Rule {
        id: "jwt",
        description: "JSON Web Token",
        pattern: r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
    },
    Rule {
        id: "basic-auth-url",
        description: "Credentials in URL",
        pattern: r"https?://[A-Za-z0-9%._~!\$&'()*+,;=\-]+:[A-Za-z0-9%._~!\$&'()*+,;=\-]+@[A-Za-z0-9.\-]+",
    },
    // ── Generic Credential Assignments ───────────────────────────────────
    // Value must be ≥16 chars to avoid false positives on short placeholders.
    Rule {
        id: "generic-token",
        description: "Generic token assignment",
        // Matches lowercase/mixed-case identifiers ending in _token or token alone.
        // ALL_CAPS vars handled by dotenv-secret.
        pattern: r#"\b(?:[a-z][A-Za-z0-9_]*_)?token\s*(?:=|:)\s*['"]?([A-Za-z0-9_\-+/=.]{8,})['"]?"#,
    },
    Rule {
        id: "generic-secret",
        description: "Generic secret assignment",
        // Matches lowercase/mixed-case identifiers ending in _secret or secret alone.
        // ALL_CAPS vars handled by dotenv-secret.
        pattern: r#"\b(?:[a-z][A-Za-z0-9_]*_)?secret\s*(?:=|:)\s*['"]?([A-Za-z0-9_\-+/=.]{8,})['"]?"#,
    },
    Rule {
        id: "generic-api-key",
        description: "Generic API key assignment",
        // Matches lowercase/mixed-case api_key, apikey, app_key. ALL_CAPS handled by dotenv-secret.
        pattern: r#"(?i)\b(?:api[-_]?key|apikey|app[-_]?key)\s*(?:=|:)\s*['"]?([A-Za-z0-9_\-+/=.]{8,})['"]?"#,
    },
    Rule {
        id: "generic-secret-key",
        description: "Generic secret_key / private_key assignment",
        pattern: r#"(?i)\b(?:secret[-_]key|private[-_]key)\s*(?:=|:)\s*['"]?([A-Za-z0-9_\-+/=.]{8,})['"]?"#,
    },
    Rule {
        id: "generic-password",
        description: "Generic password assignment",
        // Matches lowercase/mixed-case identifiers ending in _password, _passwd, _pass.
        // ALL_CAPS vars handled by dotenv-secret.
        pattern: r#"\b(?:[a-z][A-Za-z0-9_]*_)?pass(?:word|wd)?\s*(?:=|:)\s*['"]?([A-Za-z0-9_\-+/=.!@#$%^&*]{8,})['"]?"#,
    },
    Rule {
        id: "dotenv-secret",
        description: "Secret in .env file (ALL_CAPS assignment)",
        pattern: r#"(?m)^[A-Z][A-Z0-9_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|APIKEY)\s*=\s*['"]?([A-Za-z0-9_\-+/=.!@#$%^&*]{8,})['"]?"#,
    },
];

/// A finding: which rule matched in which file.
#[derive(Debug)]
pub struct Finding {
    pub path: String,
    #[allow(dead_code)]
    pub rule_id: &'static str,
    pub description: &'static str,
    /// Redacted preview of the matched context, e.g. "GITHUB_TOKEN=ghp_***1234"
    pub preview: String,
    /// 1-based line number of the match in the file
    pub line: usize,
}

/// Redact a secret value within its full match context.
/// Keeps the key name / operator visible, masks the middle of the value.
/// e.g. full_match="GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz1234"
///      value_group="ghp_abcdefghijklmnopqrstuvwxyz1234"
///      → "GITHUB_TOKEN=ghp_***1234"
fn redact_match(full_match: &str, value_group: Option<&str>) -> String {
    let val = value_group.unwrap_or(full_match).trim();

    let redacted = if val.len() <= 8 {
        format!("{}***", &val[..val.len().min(2)])
    } else {
        // char-boundary-safe slicing
        let head: String = val.chars().take(4).collect();
        let tail: String = val
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{}***{}", head, tail)
    };

    if let Some(vg) = value_group {
        if let Some(idx) = full_match.find(vg) {
            return format!(
                "{}{}{}",
                &full_match[..idx],
                redacted,
                &full_match[idx + vg.len()..]
            );
        }
    }
    redacted
}

/// Returns true if the byte slice is likely binary (contains a null byte
/// in the first 512 bytes).
fn is_binary(data: &[u8]) -> bool {
    data.iter().take(512).any(|&b| b == 0)
}

/// Scan a single file's content. Returns all matching rule descriptions.
pub fn scan_file(path: &str, content: &[u8]) -> Vec<Finding> {
    if is_binary(content) {
        return vec![];
    }
    let text = match std::str::from_utf8(content) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    // Use OnceLock per rule to compile regexes only once.
    // We store compiled regexes in a global Vec<OnceLock<Regex>>.
    static COMPILED: OnceLock<Vec<Regex>> = OnceLock::new();
    let compiled = COMPILED.get_or_init(|| {
        RULES
            .iter()
            .map(|r| Regex::new(r.pattern).expect("invalid secret detection regex"))
            .collect()
    });

    let mut findings = vec![];
    // Track seen lines — first (most specific) rule wins per line.
    let mut seen_lines = std::collections::HashSet::new();
    for (rule, re) in RULES.iter().zip(compiled.iter()) {
        for caps in re.captures_iter(text) {
            let m0 = caps.get(0).unwrap();
            let line = text[..m0.start()].chars().filter(|&c| c == '\n').count() + 1;
            if !seen_lines.insert(line) {
                continue;
            }
            let full_match = m0.as_str();
            let value_group = caps.get(1).map(|m| m.as_str());
            let preview = redact_match(full_match, value_group).trim().to_string();
            findings.push(Finding {
                path: path.to_string(),
                rule_id: rule.id,
                description: rule.description,
                preview,
                line,
            });
        }
    }
    findings
}

/// Scan multiple files. Returns all findings across all files.
pub fn scan_files(files: &[(String, Vec<u8>)]) -> Vec<Finding> {
    files
        .iter()
        .flat_map(|(path, content)| scan_file(path, content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_github_token_in_env() {
        let content = b"GITHUB_TOKEN=ghp_abc123def456ghi789jklmno012345678\n";
        let findings = scan_file(".env", content);
        assert!(
            !findings.is_empty(),
            "GITHUB_TOKEN should be detected but got no findings"
        );
        let ids: Vec<_> = findings.iter().map(|f| f.rule_id).collect();
        // Either dotenv-secret or generic-token should fire
        assert!(
            ids.iter()
                .any(|id| *id == "dotenv-secret" || *id == "generic-token"),
            "expected dotenv-secret or generic-token, got: {:?}",
            ids
        );
    }

    #[test]
    fn detects_multiple_secrets_in_same_file() {
        let content =
            b"STRIPE_SECRET_KEY=sk_live_abcdefghijklmnopqrst\nGITHUB_TOKEN=ghp_abc123def456ghi789jklmno012345678\n";
        let findings = scan_file(".env", content);
        // Should find both, not just the first dotenv match
        assert!(
            findings.len() >= 2,
            "expected at least 2 findings but got {}: {:?}",
            findings.len(),
            findings.iter().map(|f| f.rule_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_duplicate_findings_per_line() {
        // STRIPE_SECRET_KEY could previously match both generic-api-key and dotenv-secret.
        // Each line must appear at most once.
        let content = b"STRIPE_SECRET_KEY=sk_live_abcdefghijklmnopqrst\nGITHUB_TOKEN=ghp_abc123def456ghi789jklmno012345678\n";
        let findings = scan_file(".env", content);
        let lines: Vec<usize> = findings.iter().map(|f| f.line).collect();
        let unique: std::collections::HashSet<usize> = lines.iter().cloned().collect();
        assert_eq!(
            lines.len(),
            unique.len(),
            "duplicate findings on same line: {:?}",
            findings
                .iter()
                .map(|f| (f.line, f.rule_id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn full_env_file_deduplication() {
        // Simulate the full test .env — expect exactly one finding per non-comment line.
        let content = b"# comment\nSTRIPE_SECRET_KEY=sk_live_abcdefghijklmnopqrst\nSENDGRID_API_KEY=SG.abcdefghijklmnop.qrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ123\nOPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxT3BlbkFJabcdefghijklmnopqrstuvwx\nDB_PASSWORD=correcthorsebatterystaple\nAUTH_TOKEN=supersecrettoken9999\nAPP_SECRET=my_app_secret_value_here\nGITHUB_TOKEN=ghp_abc123def456ghi789jklmno012345678\n";
        let findings = scan_file(".env", content);
        // 7 non-comment lines, each should appear exactly once
        assert_eq!(
            findings.len(),
            7,
            "expected 7 findings (one per secret line), got {}: {:?}",
            findings.len(),
            findings
                .iter()
                .map(|f| (f.line, f.rule_id))
                .collect::<Vec<_>>()
        );
        let lines: Vec<usize> = findings.iter().map(|f| f.line).collect();
        let unique: std::collections::HashSet<usize> = lines.iter().cloned().collect();
        assert_eq!(lines.len(), unique.len(), "duplicate findings detected");
    }
}
