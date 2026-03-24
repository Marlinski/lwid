/**
 * detect-secrets.js — Scan files for secrets before upload.
 *
 * Pure ES module, no dependencies beyond secret-rules.js.
 */

import { isBinary, SECRET_RULES } from './secret-rules.js';

/**
 * Redact a secret value in-place within the full match string.
 * Keeps the key name / operator visible, masks the middle of the value.
 *
 * @param {string} fullMatch  — the entire regex match (match[0])
 * @param {string|undefined} valueGroup — capture group 1 (the secret value), if any
 * @returns {string}
 */
function redactMatch(fullMatch, valueGroup) {
  const val = (valueGroup ?? fullMatch).trim();

  let redacted;
  if (val.length <= 8) {
    redacted = val.slice(0, 2) + '***';
  } else {
    redacted = val.slice(0, 4) + '***' + val.slice(-4);
  }

  if (valueGroup) {
    const idx = fullMatch.indexOf(valueGroup);
    if (idx !== -1) {
      return fullMatch.slice(0, idx) + redacted + fullMatch.slice(idx + valueGroup.length);
    }
  }
  return redacted;
}

/**
 * Given a string and a character offset, return the 1-based line number.
 * @param {string} text
 * @param {number} offset
 * @returns {number}
 */
function lineNumberAt(text, offset) {
  let line = 1;
  for (let i = 0; i < offset; i++) {
    if (text[i] === '\n') line++;
  }
  return line;
}

/**
 * Scan a single file's content for secrets.
 *
 * @param {string} path — file path (used only for display)
 * @param {Uint8Array} bytes — raw file content
 * @returns {{ id: string, severity: string, preview: string, line: number }[]}
 */
export function scanFile(path, bytes) {
  if (isBinary(bytes)) return [];

  const text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
  const findings = [];
  // Track which lines already have a finding — first (most specific) rule wins.
  const seenLines = new Set();

  for (const rule of SECRET_RULES) {
    rule.pattern.lastIndex = 0;
    let match;
    while ((match = rule.pattern.exec(text)) !== null) {
      const line = lineNumberAt(text, match.index);
      if (seenLines.has(line)) continue;
      seenLines.add(line);
      const preview = redactMatch(match[0], match[1]).trim();
      findings.push({
        id: rule.id,
        severity: rule.severity,
        preview,
        line,
      });
    }
    rule.pattern.lastIndex = 0;
  }

  return findings;
}

/**
 * Scan multiple files and aggregate findings by file.
 *
 * @param {Array<{ path: string, bytes: Uint8Array }>} files
 * @returns {{ path: string, findings: { id: string, description: string, preview: string, line: number }[] }[]}
 *   Only files with at least one finding are included.
 */
export function scanFiles(files) {
  const results = [];
  for (const { path, bytes } of files) {
    const findings = scanFile(path, bytes);
    if (findings.length > 0) {
      results.push({ path, findings });
    }
  }
  return results;
}
