/**
 * Project-name normalisation (shell/js/manifest.js).
 *
 * The cases live in tests/fixtures/project-name-cases.json and are asserted
 * by BOTH this file and a Rust test in crates/lwid-cli/src/push.rs. A name
 * set in the browser and one set by `lwid push --name` end up in the same
 * manifest field, so the two implementations have to agree character for
 * character — and the only way to keep two hand-written implementations in
 * step is to make them answer the same questions.
 *
 *   node --test tests/*.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

import { sanitizeName, MAX_NAME_LENGTH } from '../shell/js/manifest.js';

const cases = JSON.parse(
  fs.readFileSync(fileURLToPath(new URL('./fixtures/project-name-cases.json', import.meta.url)), 'utf8'),
);

test('sanitizeName matches the shared fixture', () => {
  for (const { input, expected } of cases) {
    assert.equal(sanitizeName(input), expected, `input: ${JSON.stringify(input)}`);
  }
});

test('sanitizeName rejects non-strings rather than throwing', () => {
  for (const bad of [null, undefined, 42, {}, []]) {
    assert.equal(sanitizeName(bad), null);
  }
});

test('sanitizeName caps length by code point, never splitting a surrogate pair', () => {
  assert.equal(Array.from(sanitizeName('x'.repeat(500))).length, MAX_NAME_LENGTH);

  // String.slice would cut this emoji in half, leaving a lone surrogate that
  // TextEncoder then stores as U+FFFD — and Rust, which caps by code point,
  // would disagree about the result.
  for (const input of ['x'.repeat(79) + '\u{1F600}', '\u{1F600}'.repeat(100)]) {
    const out = sanitizeName(input);
    assert.equal(Array.from(out).length, MAX_NAME_LENGTH);
    for (const unit of out) {
      const c = unit.charCodeAt(0);
      assert.ok(
        !(c >= 0xd800 && c <= 0xdfff) || unit.length === 2,
        `lone surrogate in ${JSON.stringify(out)}`,
      );
    }
    // Survives a UTF-8 round trip unchanged — no replacement characters.
    assert.equal(new TextDecoder().decode(new TextEncoder().encode(out)), out);
  }
});

test('sanitizeName does not leave a trailing space when the cap lands on one', () => {
  assert.equal(sanitizeName('x'.repeat(79) + ' tail'), 'x'.repeat(79));
});

test('sanitizeName does NOT escape HTML — that is the renderer\'s job', () => {
  // Deliberate: normalisation and escaping are separate concerns, and
  // conflating them would leave a name that looks safe but is not, depending
  // on where it is used. The dropdown escapes at render (escapeHtml in
  // index.html); the toolbar uses textContent.
  const payload = '<img src=x onerror=alert(1)>';
  assert.equal(sanitizeName(payload), payload);
});
