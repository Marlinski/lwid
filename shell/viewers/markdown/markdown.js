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

const { toast, escapeHtml, resolvePath, theme, sidebarToggle } = window.LwidUI;
const Host = window.LwidHost;
const Md = window.LwidMd;

// Apply any stored light/dark override before the first paint.
theme.apply();

const $ = (id) => document.getElementById(id);
const $sidebar = $('sidebar');
const $main = $('main');
const sidebar = sidebarToggle($sidebar);

// Elements inside <main> are recreated when toggling the editor — query fresh.
const content = () => $('content');
const tocNav = () => $('toc');
const tocList = () => $('toc-list');

const state = {
  docs: [],       // markdown paths
  order: [],      // sidebar entries: { path, label, indent } | { group }
  current: null,
  title: 'Docs',
  canEdit: false,
  editing: false,
  editDirty: false, // has the editor's textarea diverged from the saved raw?
  saving: false,
  raw: '',
};

// Edit / Save / Cancel live in the shell's toolbar (see js/viewers.js +
// LWID_TOOLBAR_SET) rather than a second bar in here. The document title
// goes through LWID_TITLE_SET (see setTitle calls below), not a toolbar
// item — it's shell-owned, next to Source, the same for every viewer.
// The sidebar toggle *is* a toolbar item — below viewer.css's mobile
// breakpoint the doc nav goes off-canvas (nowhere to put a fixed-width
// sidebar on a phone), so it needs its own way back open.
function syncToolbar() {
  const items = [sidebar.toolbarItem];
  if (state.canEdit) {
    if (state.editing) {
      items.push({
        kind: 'button', id: 'save',
        label: state.saving ? 'Saving…' : 'Save',
        // Grey/plain until the text actually differs from what's saved —
        // then the same flowing-gradient CTA as the homepage's quick starts.
        variant: state.saving ? undefined : (state.editDirty ? 'cta' : undefined),
        disabled: state.saving,
      });
      items.push({ kind: 'button', id: 'cancel', label: 'Cancel', disabled: state.saving });
    } else {
      items.push({ kind: 'button', id: 'edit', label: 'Edit', title: 'Edit this document' });
    }
  }
  items.push(theme.toolbarItem());
  Host.setToolbar(items);
}

Host.onToolbarClick((id) => {
  if (id === 'edit') enterEdit();
  else if (id === 'save') saveEdit();
  else if (id === 'cancel') { const d = state.current; exitEdit(); openDoc(d); }
  else if (id === 'theme') { theme.toggle(); syncToolbar(); }
  else if (id === 'sidebar') sidebar.toggle();
});

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
  sidebar.close(); // no-op on desktop; on mobile, picking a doc should close the overlay

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

  state.title = meta.title || firstHeadingText(headings) || titleFromPath(doc);
  document.title = state.title;
  Host.setTitle(state.title);
  syncToolbar();

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
    if (resolved != null) img.src = '/' + enc(resolved);
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
      a.href = '/' + enc(resolved);
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
  state.editDirty = false;
  syncToolbar();

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
  const update = () => {
    $preview.innerHTML = Md.render($src.value).html;
    const dirty = $src.value !== state.raw;
    if (dirty !== state.editDirty) { state.editDirty = dirty; syncToolbar(); }
  };
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
  renderReaderShell();
  syncToolbar();
}

async function saveEdit() {
  const next = $('md-src').value;
  state.saving = true;
  syncToolbar();
  try {
    await Host.saveVersion([{ path: state.current, content: next }]);
    toast('Published new version');
    state.saving = false;
    exitEdit();
    await openDoc(state.current);
  } catch (err) {
    toast('Save failed: ' + err.message);
    state.saving = false;
    syncToolbar();
  }
}

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

  if (state.docs.length === 0) {
    state.title = 'Docs';
    Host.setTitle(state.title);
    syncToolbar();
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
