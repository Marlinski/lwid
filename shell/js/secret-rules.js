/**
 * secret-rules.js — Regex patterns for detecting secrets in file content.
 *
 * Patterns sourced from gitleaks/gitleaks (config/gitleaks.toml) and
 * secretlint. Designed to run entirely in the browser with no dependencies.
 */

/**
 * Returns true if the byte buffer is likely binary (contains a null byte
 * in the first 512 bytes). Binary files are skipped during scanning.
 * @param {Uint8Array} bytes
 * @returns {boolean}
 */
export function isBinary(bytes) {
  const sample = bytes.subarray(0, 512);
  for (let i = 0; i < sample.length; i++) {
    if (sample[i] === 0) return true;
  }
  return false;
}

/**
 * Each rule has:
 *   id       — unique identifier
 *   severity — 'red' | 'yellow' | 'blue'
 *              red    = known credential with specific format (high confidence)
 *              yellow = named credential assignment (medium confidence)
 *              blue   = heuristic / catch-all (low confidence)
 *   pattern  — RegExp with /g flag (lastIndex is reset before each use)
 */
export const SECRET_RULES = [
  // ── Cloud Providers ──────────────────────────────────────────────────
  {
    id: 'aws-access-key-id',
    severity: 'red',
    pattern: /\b((?:A3T[A-Z0-9]|AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16})\b/g,
  },
  {
    id: 'aws-secret-access-key',
    severity: 'red',
    pattern: /["']?(?:AWS|aws|Aws)?_?(?:SECRET|secret)_?(?:ACCESS|access)?_?(?:KEY|key)["']?\s*(?::|=|=>)\s*["']?([A-Za-z0-9/+=]{40})["']?/g,
  },
  {
    id: 'gcp-api-key',
    severity: 'red',
    pattern: /\b(AIza[A-Za-z0-9_\-]{35})\b/g,
  },
  {
    id: 'azure-ad-client-secret',
    severity: 'red',
    pattern: /(?:^|['"`\s>=:(,)])([a-zA-Z0-9_~.]{3}\dQ~[a-zA-Z0-9_~.\-]{31,34})(?:$|['"`\s<),])/gm,
  },

  // ── Source Control ────────────────────────────────────────────────────
  {
    id: 'github-token',
    severity: 'red',
    pattern: /\b(gh[pousr]_[A-Za-z0-9_]{36})\b/g,
  },
  {
    id: 'github-fine-grained-token',
    severity: 'red',
    pattern: /\b(github_pat_[A-Za-z0-9_]{82})\b/g,
  },
  {
    id: 'gitlab-pat',
    severity: 'red',
    pattern: /\b(glpat-[A-Za-z0-9\-_]{20})\b/g,
  },

  // ── AI / LLM Providers ───────────────────────────────────────────────
  {
    id: 'openai-api-key',
    severity: 'red',
    pattern: /\b(sk-(?:proj|svcacct|admin)-[A-Za-z0-9_\-]{58,74}T3BlbkFJ[A-Za-z0-9_\-]{58,74}|sk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20})\b/g,
  },
  {
    id: 'anthropic-api-key',
    severity: 'red',
    pattern: /\b(sk-ant-api03-[A-Za-z0-9_\-]{93}AA)\b/g,
  },

  // ── Payment ───────────────────────────────────────────────────────────
  {
    id: 'stripe-api-key',
    severity: 'red',
    pattern: /\b((?:sk|rk)_(?:live|test)_[A-Za-z0-9]{20,247})\b/g,
  },
  {
    id: 'square-access-token',
    severity: 'red',
    pattern: /\b(sqOatp-[0-9A-Za-z\-_]{22})\b/g,
  },
  {
    id: 'paypal-braintree-token',
    severity: 'red',
    pattern: /\baccess_token\$production\$[0-9a-z]{16}\$[0-9a-f]{32}\b/g,
  },

  // ── Communication ─────────────────────────────────────────────────────
  {
    id: 'sendgrid-api-key',
    severity: 'red',
    pattern: /\b(SG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43})\b/g,
  },
  {
    id: 'twilio-api-key',
    severity: 'red',
    pattern: /\b(SK[0-9a-fA-F]{32})\b/g,
  },
  {
    id: 'twilio-account-sid',
    severity: 'yellow',
    pattern: /\b(AC[a-z0-9]{32})\b/g,
  },
  {
    id: 'mailchimp-api-key',
    severity: 'red',
    pattern: /\b([0-9a-f]{32}-us[0-9]{1,2})\b/g,
  },
  {
    id: 'mailgun-api-key',
    severity: 'red',
    pattern: /\b(key-[0-9a-zA-Z]{32})\b/g,
  },

  // ── Infrastructure ────────────────────────────────────────────────────
  {
    id: 'slack-token',
    severity: 'red',
    pattern: /\b(xox[bpaor]-(?:\d+-)?(?:[A-Za-z0-9]{1,40}-)+[A-Za-z0-9]{1,40})\b/g,
  },
  {
    id: 'slack-webhook',
    severity: 'red',
    pattern: /https:\/\/hooks\.slack\.com\/services\/T[A-Za-z0-9]{1,40}\/B[A-Za-z0-9]{1,40}\/[A-Za-z0-9]{1,40}/gi,
  },
  {
    id: 'digitalocean-pat',
    severity: 'red',
    pattern: /\b(dop_v1_[a-f0-9]{64})\b/g,
  },
  {
    id: 'databricks-token',
    severity: 'red',
    pattern: /\b(dapi[a-f0-9]{32}(?:-\d)?)\b/g,
  },

  // ── Cryptographic Material ────────────────────────────────────────────
  {
    id: 'private-key-pem',
    severity: 'red',
    pattern: /-----BEGIN[ ]?(?:(?:RSA|DSA|EC|OPENSSH|PGP) )?PRIVATE KEY(?: BLOCK)?-----/gm,
  },

  // ── Generic High-Signal ───────────────────────────────────────────────
  {
    id: 'jwt',
    severity: 'yellow',
    pattern: /\b(eyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,})\b/g,
  },
  {
    id: 'basic-auth-url',
    severity: 'yellow',
    pattern: /https?:\/\/[A-Za-z0-9%._~!$&'()*+,;=\-]+:[A-Za-z0-9%._~!$&'()*+,;=\-]+@[A-Za-z0-9.\-]+/gi,
  },

  // ── Generic Credential Assignments ────────────────────────────────────
  {
    id: 'generic-token',
    severity: 'yellow',
    pattern: /\b(?:[a-z][A-Za-z0-9_]*_)?token\s*(?:=|:)\s*['"`]?([A-Za-z0-9_\-+/=.]{8,})['"`]?/g,
  },
  {
    id: 'generic-secret',
    severity: 'yellow',
    pattern: /\b(?:[a-z][A-Za-z0-9_]*_)?secret\s*(?:=|:)\s*['"`]?([A-Za-z0-9_\-+/=.]{8,})['"`]?/g,
  },
  {
    id: 'generic-api-key',
    severity: 'yellow',
    pattern: /\b(?:api[-_]?key|apikey|app[-_]?key)\s*(?:=|:)\s*['"`]?([A-Za-z0-9_\-+/=.]{8,})['"`]?/gi,
  },
  {
    id: 'generic-secret-key',
    severity: 'yellow',
    pattern: /\b(?:secret[-_]key|private[-_]key)\s*(?:=|:)\s*['"`]?([A-Za-z0-9_\-+/=.]{8,})['"`]?/gi,
  },
  {
    id: 'generic-password',
    severity: 'yellow',
    pattern: /\b(?:[a-z][A-Za-z0-9_]*_)?pass(?:word|wd)?\s*(?:=|:)\s*['"`]?([A-Za-z0-9_\-+/=.!@#$%^&*]{8,})['"`]?/g,
  },
  {
    id: 'dotenv-secret',
    severity: 'blue',
    // Matches uppercase env var names ending in TOKEN, SECRET, KEY, PASSWORD, PASS, CREDENTIAL
    pattern: /^[A-Z][A-Z0-9_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|APIKEY)\s*=\s*['"`]?([A-Za-z0-9_\-+/=.!@#$%^&*]{8,})['"`]?/gm,
  },
];
