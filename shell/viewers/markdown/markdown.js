/**
 * markdown.js — the lwid docs viewer.
 *
 * Renders a project's Markdown files as a small documentation site: a sidebar
 * (honouring SUMMARY.md / _sidebar.md when present), relative links resolved
 * to in-viewer navigation, a per-page table of contents, and — for editable
 * links — inline editing that publishes a new project version.
 *
 * viewer-ui.js / viewer-host.js / md.js are loaded as classic scripts by
 * index.html; this module just drives them.
 */

const { toast, escapeHtml, resolvePath } = window.LwidUI;
const Host = window.LwidHost;
const Md = window.LwidMd;

const $ = (id) => document.getElementById(id);
const $sidebar = $('sidebar');
const $title = $('title');
const $main = $('main');
const $editBtn = $('edit-btn');
const $saveBtn = $('save-btn');
const $cancelBtn = $('cancel-btn');

// Elements inside <main> are recreated when toggling the editor — query fresh.
const content = () => $('content');
const tocNav = () => $('toc');
const tocList = () => $('toc-list');

const state = {
  docs: [],       // markdown paths
  order: [],      // sidebar entries: { path, label, indent } | { group }
  current: null,
  canEdit: false,
  editing: false,
  raw: '',
};

const isMd = (p) => /\.(md|markdown)$/i.test(p);
const base = (p) => p.split('/').pop();
const dirOf = (p) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '');
const enc = (p) => p.split('/').map(encodeURIComponent).join('/');

function titleFromPath(p) {
  return base(p)
    .replace(/\.(md|markdown)$/i, '')
    .replace(/[-_]+/g, ' ')
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Resolve `target` (project-relative) to an actual doc path, or null. */
function matchDoc(target) {
  if (!target) return null;
  const norm = target.replace(/^\.\//, '').replace(/^\//, '');
  if (state.docs.includes(norm)) return norm;
  const withMd = isMd(norm) ? norm : norm + '.md';
  if (state.docs.includes(withMd)) return withMd;
  const b = base(norm).toLowerCase();
  return state.docs.find((d) => base(d).toLowerCase() === b) || null;
}

// ── Main shell (re-created around the editor) ─────────────────────────────

function renderReaderShell() {
  $main.innerHTML = `
    <div class="md-layout">
      <article class="md-doc"><div class="md-content" id="content"></div></article>
      <nav class="md-toc" id="toc" hidden>
        <div class="md-toc__title">On this page</div>
        <div id="toc-list"></div>
      </nav>
    </div>`;
  $main.addEventListener('scroll', onScroll, { passive: true });
}

// ── Sidebar ──────────────────────────────────────────────────────────────

async function buildSidebar() {
  const summaryPath = state.docs.find((d) => /(^|\/)(SUMMARY|_sidebar|_toc)\.md$/i.test(d));
  state.order = [];

  if (summaryPath) {
    try {
      const text = await Host.readText(summaryPath);
      const sdir = dirOf(summaryPath);
      const re = /^(\s*)[-*+]\s+\[([^\]]+)\]\(([^)]+)\)/gm;
      let m;
      while ((m = re.exec(text))) {
        const indent = Math.floor(m[1].replace(/\t/g, '  ').length / 2);
        const rawTarget = m[3].trim().split('#')[0];
        if (!rawTarget) continue;
        const target = (sdir ? sdir + '/' : '') + rawTarget.replace(/^\.\//, '');
        const doc = matchDoc(target);
        if (doc) state.order.push({ path: doc, label: m[2].trim(), indent });
      }
    } catch (_) { /* fall through */ }
  }

  if (state.order.length === 0) {
    const readme = state.docs.find((d) => /(^|\/)readme\.(md|markdown)$/i.test(d));
    const rest = state.docs.filter((d) => d !== readme && d !== summaryPath).sort();
    const ordered = readme ? [readme, ...rest] : rest;
    let lastDir = null;
    for (const p of ordered) {
      const d = dirOf(p);
      if (d && d !== lastDir) state.order.push({ group: d });
      lastDir = d;
      state.order.push({ path: p, label: titleFromPath(p), indent: 0 });
    }
  }

  renderSidebar();
}

function renderSidebar() {
  $sidebar.innerHTML = '';
  for (const entry of state.order) {
    if (entry.group) {
      const g = document.createElement('div');
      g.className = 'v-nav__group';
      g.textContent = entry.group + '/';
      $sidebar.appendChild(g);
      continue;
    }
    const b = document.createElement('button');
    b.className = 'v-nav__item' + (entry.path === state.current ? ' v-nav__item--active' : '');
    b.style.paddingLeft = 14 + (entry.indent || 0) * 14 + 'px';
    b.textContent = entry.label;
    b.addEventListener('click', () => openDoc(entry.path));
    $sidebar.appendChild(b);
  }
}

// ── Rendering ────────────────────────────────────────────────────────────

async function openDoc(path, anchor) {
  if (state.editing) return;
  const doc = matchDoc(path) || path;
  if (!state.docs.includes(doc)) { toast('Not found: ' + path); return; }
  state.current = doc;
  renderSidebar();

  content().innerHTML = '<p class="v-empty"><span class="v-spinner"></span></p>';
  let src;
  try {
    src = await Host.readText(doc);
  } catch (err) {
    content().innerHTML = `<p class="v-empty">Could not read ${escapeHtml(doc)}<br>${escapeHtml(err.message)}</p>`;
    return;
  }
  state.raw = src;
  const { html, meta, headings } = Md.render(src);
  content().innerHTML = html;

  $title.textContent = meta.title || firstHeadingText(headings) || titleFromPath(doc);
  document.title = $title.textContent;

  rewriteLinks(doc);
  buildToc(headings);

  const target = anchor && document.getElementById(anchor);
  if (target) target.scrollIntoView();
  else $main.scrollTop = 0;

  try { location.hash = enc(doc) + (anchor ? '#' + anchor : ''); } catch (_) { /* ignore */ }
}

function firstHeadingText(headings) {
  const h1 = headings.find((h) => h.level === 1);
  return h1 ? h1.text : null;
}

function rewriteLinks(fromPath) {
  const root = content();
  for (const img of root.querySelectorAll('img[src]')) {
    const resolved = resolvePath(fromPath, img.getAttribute('src'));
    if (resolved != null) img.src = '/sandbox/' + enc(resolved);
  }
  for (const a of root.querySelectorAll('a[href]')) {
    const href = a.getAttribute('href');
    if (!href) continue;
    if (href.startsWith('#')) {
      a.addEventListener('click', (e) => {
        e.preventDefault();
        const el = document.getElementById(decodeURIComponent(href.slice(1)));
        if (el) el.scrollIntoView({ behavior: 'smooth' });
      });
      continue;
    }
    if (/^[a-z][a-z0-9+.-]*:/i.test(href)) continue; // external / mailto
    const [rel, hash] = href.split('#');
    const resolved = resolvePath(fromPath, rel || '');
    if (resolved == null) continue;
    const doc = matchDoc(resolved);
    if (doc) {
      a.addEventListener('click', (e) => { e.preventDefault(); openDoc(doc, hash); });
    } else {
      a.href = '/sandbox/' + enc(resolved);
      a.target = '_blank';
      a.rel = 'noopener';
    }
  }
}

// ── Table of contents ────────────────────────────────────────────────────

function buildToc(headings) {
  const items = headings.filter((h) => h.level >= 2 && h.level <= 4);
  const list = tocList();
  if (!list || items.length < 2) { if (tocNav()) tocNav().hidden = true; return; }
  list.innerHTML = '';
  for (const h of items) {
    const a = document.createElement('a');
    a.href = '#' + h.slug;
    a.dataset.level = h.level;
    a.dataset.slug = h.slug;
    a.textContent = h.text;
    a.addEventListener('click', (e) => {
      e.preventDefault();
      const el = document.getElementById(h.slug);
      if (el) el.scrollIntoView({ behavior: 'smooth' });
    });
    list.appendChild(a);
  }
  tocNav().hidden = false;
}

function onScroll() {
  const list = tocList();
  if (!list) return;
  const links = [...list.querySelectorAll('a')];
  let active = null;
  for (const link of links) {
    const el = document.getElementById(link.dataset.slug);
    if (el && el.getBoundingClientRect().top < 120) active = link.dataset.slug;
  }
  for (const link of links) link.classList.toggle('md-toc--active', link.dataset.slug === active);
}

// ── Editing ──────────────────────────────────────────────────────────────

function enterEdit() {
  state.editing = true;
  $editBtn.hidden = true;
  $saveBtn.hidden = false;
  $cancelBtn.hidden = false;

  $main.innerHTML = `
    <div class="md-editor">
      <textarea spellcheck="false" id="md-src"></textarea>
      <div class="md-editor__preview">
        <article class="md-doc"><div class="md-content" id="md-preview"></div></article>
      </div>
    </div>`;
  const $src = $('md-src');
  const $preview = $('md-preview');
  $src.value = state.raw;
  const update = () => { $preview.innerHTML = Md.render($src.value).html; };
  update();
  $src.addEventListener('input', update);
  $src.addEventListener('keydown', (e) => {
    if (e.key === 'Tab') {
      e.preventDefault();
      const s = $src.selectionStart;
      $src.value = $src.value.slice(0, s) + '  ' + $src.value.slice($src.selectionEnd);
      $src.selectionStart = $src.selectionEnd = s + 2;
      update();
    }
  });
  $src.focus();
}

function exitEdit() {
  state.editing = false;
  $saveBtn.hidden = true;
  $cancelBtn.hidden = true;
  $editBtn.hidden = false;
  renderReaderShell();
}

async function saveEdit() {
  const next = $('md-src').value;
  $saveBtn.disabled = true;
  $saveBtn.textContent = 'Saving…';
  try {
    await Host.saveVersion([{ path: state.current, content: next }]);
    toast('Published new version');
    exitEdit();
    await openDoc(state.current);
  } catch (err) {
    toast('Save failed: ' + err.message);
  } finally {
    $saveBtn.disabled = false;
    $saveBtn.textContent = 'Save';
  }
}

$editBtn.addEventListener('click', enterEdit);
$saveBtn.addEventListener('click', saveEdit);
$cancelBtn.addEventListener('click', () => { const d = state.current; exitEdit(); openDoc(d); });
$('menu-toggle').addEventListener('click', () => { $sidebar.hidden = !$sidebar.hidden; });

// ── Boot ─────────────────────────────────────────────────────────────────

(async () => {
  renderReaderShell();

  let manifest;
  try {
    manifest = await Host.manifest();
  } catch (err) {
    content().innerHTML = `<p class="v-empty">Failed to load project.<br>${escapeHtml(err.message)}</p>`;
    return;
  }
  state.canEdit = !!manifest.canEdit;
  state.docs = manifest.files.map((f) => f.path).filter(isMd);

  if (state.canEdit) $editBtn.hidden = false;
  else $('read-only').hidden = false;

  if (state.docs.length === 0) {
    content().innerHTML = '<p class="v-empty">No Markdown documents in this project.</p>';
    return;
  }

  await buildSidebar();

  const hash = decodeURIComponent(location.hash.replace(/^#/, ''));
  const [hDoc, hAnchor] = hash.split('#');
  const start = (hDoc && matchDoc(hDoc))
    || state.order.find((e) => e.path)?.path
    || state.docs[0];
  await openDoc(start, hAnchor);
})();
