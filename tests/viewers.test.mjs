/**
 * Tests for viewer detection (shell/js/viewers.js).
 *
 *   node --test tests/
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { detectViewer, knownViewer, FALLBACK_VIEWER } from '../shell/js/viewers.js';

test('a project with index.html is a plain site (no viewer)', () => {
  assert.equal(detectViewer(['index.html', 'style.css', 'app.js']), null);
});

test('any HTML file anywhere disables viewer selection', () => {
  assert.equal(detectViewer(['docs/guide.md', 'preview.html']), null);
  assert.equal(detectViewer(['nested/deep/page.htm']), null);
});

test('a lone notebook picks the notebook viewer', () => {
  assert.equal(detectViewer(['Analysis.ipynb']), 'notebook');
});

test('notebook wins over markdown when both are present', () => {
  assert.equal(detectViewer(['README.md', 'run.ipynb']), 'notebook');
});

test('markdown files pick the docs viewer', () => {
  assert.equal(detectViewer(['README.md']), 'markdown');
  assert.equal(detectViewer(['docs/a.markdown', 'docs/b.markdown']), 'markdown');
});

test('anything else falls back to the file browser', () => {
  assert.equal(detectViewer(['data.csv', 'notes.txt']), FALLBACK_VIEWER);
  assert.equal(detectViewer(['photo.png']), FALLBACK_VIEWER);
});

test('an empty project has no viewer', () => {
  assert.equal(detectViewer([]), null);
  assert.equal(detectViewer(null), null);
});

test('detection is case-insensitive', () => {
  assert.equal(detectViewer(['NOTES.MD']), 'markdown');
  assert.equal(detectViewer(['Run.IPYNB']), 'notebook');
  assert.equal(detectViewer(['INDEX.HTML']), null);
});

test('accepts any iterable of paths', () => {
  const m = new Map([['a.ipynb', 1], ['b.txt', 2]]);
  assert.equal(detectViewer(m.keys()), 'notebook');
});

test('a valid override wins over content detection', () => {
  assert.equal(detectViewer(['data.csv'], 'markdown'), 'markdown');
  assert.equal(detectViewer(['a.ipynb'], 'files'), 'files');
});

test('an unknown override is ignored', () => {
  assert.equal(detectViewer(['a.ipynb'], 'bogus'), 'notebook');
});

test('an override never overrides a real website', () => {
  assert.equal(detectViewer(['index.html'], 'markdown'), null);
});

test('knownViewer recognises the registry and the fallback', () => {
  assert.ok(knownViewer('notebook'));
  assert.ok(knownViewer('markdown'));
  assert.ok(knownViewer('files'));
  assert.ok(!knownViewer('spreadsheet'));
});
