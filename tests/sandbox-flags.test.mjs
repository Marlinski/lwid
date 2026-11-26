/**
 * Tests that the preview's two sandbox attributes stay in step.
 *
 *   node --test tests/*.test.mjs
 *
 * Sandbox flags intersect down the frame tree: the content frame created in
 * `__lwid_sandbox__/sandbox.html` can never hold more than the outer
 * `#sandbox-frame` in `shell/index.html` grants it, and the browser drops the
 * surplus with no warning anywhere.
 *
 * That silence is the whole problem. The two attributes drifted apart once
 * already — the inner frame asked for four flags the outer did not have, so
 * `<form>` submission, `target="_blank"`, `<a download>` and `alert()` were
 * all dead in every hosted project, with nothing in the console to say why
 * (issues #14 and #10). Nothing in the code made the dependency visible, so
 * these tests make it fail loudly instead.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (rel) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');

/** Strip HTML comments so prose about `sandbox=` never matches as markup. */
const stripComments = (html) => html.replace(/<!--[\s\S]*?-->/g, '');

/** Flags on the outer `#sandbox-frame` element in the shell. */
function outerFlags() {
  const html = stripComments(read('../shell/index.html'));
  const tag = html.match(/<iframe\b[^>]*\bid="sandbox-frame"[^>]*>/);
  assert.ok(tag, 'could not find the #sandbox-frame <iframe> in shell/index.html');
  const attr = tag[0].match(/\bsandbox="([^"]*)"/);
  assert.ok(attr, '#sandbox-frame has no sandbox attribute');
  return attr[1].split(/\s+/).filter(Boolean);
}

/** Flags the bridge requests for the content frame. */
function innerFlags() {
  const js = read('../shell/__lwid_sandbox__/sandbox.html');
  const call = js.match(/setAttribute\(\s*'sandbox'\s*,\s*'([^']*)'\s*\)/);
  assert.ok(call, "could not find the content frame's setAttribute('sandbox', …)");
  return call[1].split(/\s+/).filter(Boolean);
}

test('the outer frame grants everything the content frame asks for', () => {
  const outer = new Set(outerFlags());
  const missing = innerFlags().filter((f) => !outer.has(f));
  assert.deepEqual(
    missing,
    [],
    `#sandbox-frame is missing ${missing.join(', ')}, so the content frame's ` +
      'request for them is silently dropped. Add them in shell/index.html.',
  );
});

test('the flags issues #14, #10 and #17 turn on are present on both frames', () => {
  // Pinned by name: a future edit that drops one of these would otherwise
  // still pass the superset test above by removing it from both sides.
  const required = [
    'allow-scripts',      // project code runs at all
    'allow-same-origin',  // the per-project origin, the SW, the store
    'allow-forms',        // #14 — a plain <form> submit
    'allow-popups',       // #10 — target="_blank"
    'allow-downloads',    // #10 — <a download>
    'allow-modals',       // #17 — alert()/confirm()/print() from project code
    // A new tab opened from a project is a normal tab rather than a
    // sandboxed one. Needed for PDFs (browsers render none in a sandboxed
    // context) and for external sites that break when sandboxed. It gives a
    // project nothing it lacks: the escaped tab is on the origin the project
    // already runs scripts on (its own sandbox origin, or the shell's own in
    // fallback mode, which allow-scripts + allow-same-origin already exposes).
    'allow-popups-to-escape-sandbox',
  ];
  for (const flag of required) {
    assert.ok(outerFlags().includes(flag), `outer #sandbox-frame lost ${flag}`);
    assert.ok(innerFlags().includes(flag), `content frame lost ${flag}`);
  }
});

test('both frames grant exactly the same flags', () => {
  // The superset test above is the hard requirement; this one keeps the two
  // lists identical so neither side quietly asks for something unused.
  assert.deepEqual([...outerFlags()].sort(), [...innerFlags()].sort());
});
