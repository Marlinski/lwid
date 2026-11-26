/**
 * Tests for the page part of project links (shell/js/page-link.js).
 *
 *   node --test tests/*.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { splitFragment, pageSuffix, isHomePage, describePage } from '../shell/js/page-link.js';

const READ = 'vsk6qHpx2quWdI5H6z8vqch1eEV7dsHoEI2OTH39qwo';
const WRITE = 'ywSlnn2_szioLltlberJmaWqXqTHL3vjOtU4Y1UhuBI';

test('links without a page parse exactly as before', () => {
  assert.deepEqual(splitFragment(READ), { keys: READ, page: null });
  assert.deepEqual(splitFragment('#' + READ), { keys: READ, page: null });
  assert.deepEqual(splitFragment(`${READ}:${WRITE}`), { keys: `${READ}:${WRITE}`, page: null });
});

test('a page follows the keys after the first slash', () => {
  assert.deepEqual(splitFragment(`${READ}/docs/page2.html`), {
    keys: READ, page: { path: 'docs/page2.html', query: '' },
  });
  assert.deepEqual(splitFragment(`${READ}:${WRITE}/page2.html?id=3#intro`), {
    keys: `${READ}:${WRITE}`, page: { path: 'page2.html', query: '?id=3#intro' },
  });
  assert.deepEqual(splitFragment(`${READ}/page.html#intro`), {
    keys: READ, page: { path: 'page.html', query: '#intro' },
  });
});

test('pageSuffix and splitFragment round-trip', () => {
  const pages = [
    { path: 'docs/page2.html', query: '' },
    { path: 'docs/page2.html', query: '?id=3&q=a%20b#intro' },
    { path: 'my page.html', query: '' },
    { path: 'odd#name?.html', query: '?x=1' },
    { path: 'café/ünï.html', query: '#top' },
    { path: '', query: '?tab=2' },
  ];
  for (const page of pages) {
    for (const keys of [READ, `${READ}:${WRITE}`]) {
      assert.deepEqual(splitFragment(keys + pageSuffix(page)), { keys, page }, JSON.stringify(page));
    }
  }
});

test('file names are encoded so ? and # in them stay part of the path', () => {
  assert.equal(pageSuffix({ path: 'odd#name?.html', query: '?x=1' }), '/odd%23name%3F.html?x=1');
  assert.equal(pageSuffix({ path: 'my page.html', query: '' }), '/my%20page.html');
});

test('no page means no suffix', () => {
  assert.equal(pageSuffix(null), '');
  assert.equal(pageSuffix(undefined), '');
});

test('a malformed escape in a hand-edited link is kept, not thrown', () => {
  assert.deepEqual(splitFragment(`${READ}/100%.html`).page, { path: '100%.html', query: '' });
});

test('a page path with . or .. segments is dropped, encoded or not', () => {
  for (const suffix of ['/../../api/x', '/docs/../index.html', '/./a.html', '/%2e%2e/x', '/%2E%2E', '/..?q=1']) {
    assert.deepEqual(splitFragment(READ + suffix), { keys: READ, page: null }, suffix);
  }
  // Dots inside a name are fine.
  assert.deepEqual(splitFragment(`${READ}/..hidden/a..b.html`).page, { path: '..hidden/a..b.html', query: '' });
});

test('isHomePage: the entry point (or bare index) with no query is home', () => {
  assert.equal(isHomePage(null, 'index.html'), true);
  assert.equal(isHomePage({ path: 'index.html', query: '' }, 'index.html'), true);
  assert.equal(isHomePage({ path: '', query: '' }, 'index.html'), true);
  assert.equal(isHomePage({ path: 'main.html', query: '' }, 'main.html'), true);
  assert.equal(isHomePage({ path: '', query: '' }, null), true);
});

test('isHomePage: another page, or any query or anchor, is not home', () => {
  assert.equal(isHomePage({ path: 'docs/page2.html', query: '' }, 'index.html'), false);
  assert.equal(isHomePage({ path: 'index.html', query: '?id=3' }, 'index.html'), false);
  assert.equal(isHomePage({ path: 'index.html', query: '#intro' }, 'index.html'), false);
  assert.equal(isHomePage({ path: '', query: '?tab=2' }, null), false);
});

test('describePage reads as path plus query', () => {
  assert.equal(describePage({ path: 'docs/page2.html', query: '?id=3#intro' }), 'docs/page2.html?id=3#intro');
  assert.equal(describePage(null), '');
});
