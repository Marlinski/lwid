/**
 * notebook.js — the lwid notebook viewer.
 *
 * Renders an .ipynb Colab-style: Markdown + code cells with their saved
 * outputs (no runtime needed to read one), and — on demand — a Pyodide
 * kernel so cells can actually run. Run state is mirrored into lwid.store so a
 * shared link shows the last execution; "Save .ipynb" publishes a new version.
 *
 * Filename / kernel status / Run all / Stop / Restart / Clear / Save live in
 * the shell's toolbar (LwidHost.setToolbar) rather than a bar drawn in here —
 * see syncToolbar() below.
 */
import { parseNotebook, serializeNotebook, nextId } from '/sandbox/__viewer__/nbformat.js';
import { Kernel } from '/sandbox/__viewer__/kernel.js';

const { toast, escapeHtml, resolvePath } = window.LwidUI;
const Host = window.LwidHost;
const Md = window.LwidMd;

const $ = (id) => document.getElementById(id);
const $doc = $('doc');
const $helpModal = $('help-modal');

const enc = (p) => p.split('/').map(encodeURIComponent).join('/');
const STORE_PREFIX = 'viewer:notebook:';

function hashString(s) {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

const state = {
  path: null,
  notebooks: [],       // every .ipynb in the project (file-select when > 1)
  nb: null,             // { cells, language, meta, nbformat }
  fileHash: '',
  canEdit: false,
  dirty: false,        // run-state differs from the saved file
  running: false,
  saving: false,
  restarting: false,
};

const kernel = new Kernel({ onStatus: syncToolbar });

// ── Toolbar ──────────────────────────────────────────────────────────────

const STATUS_LABEL = {
  uninitialized: 'no kernel',
  loading: 'starting Python…',
  idle: 'kernel ready',
  busy: 'running…',
  dead: 'kernel crashed',
};

function syncToolbar() {
  // The current filename goes through Host.setTitle() (shell-owned, next to
  // Source — see openNotebook()). The switcher below is a genuine control
  // (it changes which notebook is open), so it still belongs in the center.
  const items = [];

  if (state.notebooks.length > 1) {
    items.push({
      kind: 'select', id: 'file', value: state.path,
      options: state.notebooks.map((p) => ({ value: p, label: p })),
      title: 'Switch notebook',
    });
  }

  items.push({ kind: 'status', label: STATUS_LABEL[kernel.status] || kernel.status, tone: kernel.status });
  // Restart acts on the kernel the status just reported on — keep it right
  // next to that status rather than off with the other cell-level actions.
  items.push({ kind: 'button', id: 'restart', label: '⟳ Restart', title: 'Restart the kernel', disabled: state.restarting });

  if (state.running) {
    items.push({ kind: 'button', id: 'stop', label: '■ Stop', title: 'Stop execution' });
  } else {
    items.push({ kind: 'button', id: 'run-all', label: '▶▶ Run all', title: 'Run every cell' });
  }
  items.push({ kind: 'button', id: 'clear', label: 'Clear', title: 'Clear all outputs' });

  if (state.canEdit) {
    items.push({
      kind: 'button', id: 'save',
      label: state.saving ? 'Saving…' : (state.dirty ? 'Save .ipynb •' : 'Save .ipynb'),
      variant: 'primary', disabled: state.saving, title: 'Publish a new version',
    });
  }

  items.push({ kind: 'button', id: 'help', label: '?', title: 'Notebook help & keyboard shortcuts' });

  Host.setToolbar(items);
}

Host.onToolbarClick((id, value) => {
  if (id === 'file') openNotebook(value);
  else if (id === 'run-all') runAll();
  else if (id === 'stop') { kernel.interrupt(); toast('Execution stopped'); syncToolbar(); }
  else if (id === 'restart') doRestart();
  else if (id === 'clear') doClear();
  else if (id === 'save') doSave();
  else if (id === 'help') toggleHelp();
});

// ── Boot ─────────────────────────────────────────────────────────────────

(async () => {
  let manifest;
  try {
    manifest = await Host.manifest();
  } catch (err) {
    $doc.innerHTML = `<div class="v-empty">Failed to load project.<br>${escapeHtml(err.message)}</div>`;
    return;
  }
  state.canEdit = !!manifest.canEdit;
  state.notebooks = manifest.files.map((f) => f.path).filter((p) => /\.ipynb$/i.test(p));

  if (state.notebooks.length === 0) {
    $doc.innerHTML = '<div class="v-empty">No notebooks in this project.</div>';
    syncToolbar();
    return;
  }

  const preferred =
    state.notebooks.find((p) => /(^|\/)(index|main)\.ipynb$/i.test(p)) || state.notebooks[0];
  await openNotebook(preferred);
})();

// ── Load / restore ───────────────────────────────────────────────────────

async function openNotebook(path) {
  state.path = path;
  document.title = path.split('/').pop();
  Host.setTitle(document.title);
  $doc.innerHTML = '<div class="v-empty"><span class="v-spinner"></span></div>';
  syncToolbar();

  let text;
  try {
    text = await Host.readText(path);
  } catch (err) {
    $doc.innerHTML = `<div class="v-empty">Could not read ${escapeHtml(path)}<br>${escapeHtml(err.message)}</div>`;
    return;
  }

  try {
    state.nb = parseNotebook(text);
  } catch (err) {
    $doc.innerHTML = `<div class="v-empty">This file is not a valid notebook.<br>${escapeHtml(err.message)}</div>`;
    return;
  }
  state.fileHash = hashString(text);
  state.dirty = false;

  await maybeRestoreRunState();
  renderAll();
  syncToolbar();
}

async function maybeRestoreRunState() {
  if (!window.lwid) return;
  try {
    const saved = await window.lwid.store.get(STORE_PREFIX + state.path);
    if (saved && saved.fileHash === state.fileHash && Array.isArray(saved.cells)) {
      const byId = new Map(state.nb.cells.map((c) => [c.id, c]));
      saved.cells.forEach((sc, i) => {
        const cell = byId.get(sc.id) || state.nb.cells[i];
        if (!cell || cell.type !== 'code') return;
        cell.outputs = sc.outputs || [];
        cell.execCount = sc.execCount ?? null;
      });
      state.dirty = true;
    }
  } catch (_) { /* persistence is best-effort */ }
}

const persist = debounce(async () => {
  if (!window.lwid) return;
  try {
    await window.lwid.store.set(STORE_PREFIX + state.path, {
      fileHash: state.fileHash,
      savedAt: Date.now(),
      cells: state.nb.cells
        .filter((c) => c.type === 'code')
        .map((c) => ({ id: c.id, execCount: c.execCount, outputs: c.outputs })),
    });
  } catch (_) { /* ignore */ }
}, 800);

// ── Rendering ────────────────────────────────────────────────────────────

function renderAll() {
  $doc.innerHTML = '';
  for (const cell of state.nb.cells) {
    cell._el = renderCell(cell);
    $doc.appendChild(cell._el);
  }
  if (state.canEdit) $doc.appendChild(renderAddCellRow());
}

// ── Cell insert / delete ─────────────────────────────────────────────────

/** Insert a new empty cell right after `afterCell` (or at the top if null). */
function insertCell(afterCell, type) {
  const idx = afterCell ? state.nb.cells.indexOf(afterCell) : -1;
  const cell = { id: nextId(), type, source: '', metadata: {} };
  if (type === 'code') { cell.execCount = null; cell.outputs = []; }
  state.nb.cells.splice(idx + 1, 0, cell);
  markDirty();
  renderAll();
  focusCell(cell);
}

function deleteCell(cell) {
  if (state.nb.cells.length <= 1) { toast("Can't delete the only cell"); return; }
  const idx = state.nb.cells.indexOf(cell);
  if (idx === -1) return;
  state.nb.cells.splice(idx, 1);
  markDirty();
  renderAll();
  toast('Cell deleted — reload without saving to get it back');
}

function focusCell(cell) {
  requestAnimationFrame(() => {
    if (cell._inputEl) cell._inputEl.focus();
    else if (cell._el) cell._el.scrollIntoView({ block: 'center', behavior: 'smooth' });
  });
}

function renderCellControls(cell) {
  const box = document.createElement('div');
  box.className = 'nb-gutter__controls';

  const addCode = document.createElement('button');
  addCode.className = 'nb-gutter__ctrl';
  addCode.title = 'Insert code cell below';
  addCode.textContent = '+';
  addCode.addEventListener('click', () => insertCell(cell, 'code'));

  const addMd = document.createElement('button');
  addMd.className = 'nb-gutter__ctrl';
  addMd.title = 'Insert markdown cell below';
  addMd.textContent = '+M';
  addMd.addEventListener('click', () => insertCell(cell, 'markdown'));

  // Native confirm() doesn't fire in this sandbox (no allow-modals), so
  // deleting arms on the first click and only commits on a second one
  // within a couple of seconds — no blocking dialog needed.
  const del = document.createElement('button');
  del.className = 'nb-gutter__ctrl nb-gutter__ctrl--danger';
  del.title = 'Delete this cell';
  del.textContent = '×';
  let armTimer = null;
  const disarm = () => {
    clearTimeout(armTimer);
    armTimer = null;
    del.textContent = '×';
    del.title = 'Delete this cell';
    del.classList.remove('nb-gutter__ctrl--armed');
  };
  del.addEventListener('click', () => {
    if (armTimer) { disarm(); deleteCell(cell); return; }
    del.textContent = '✓';
    del.title = 'Click again to delete';
    del.classList.add('nb-gutter__ctrl--armed');
    armTimer = setTimeout(disarm, 2500);
  });
  del.addEventListener('blur', disarm);

  box.append(addCode, addMd, del);
  return box;
}

function renderAddCellRow() {
  const row = document.createElement('div');
  row.className = 'nb-add-row';
  const last = state.nb.cells[state.nb.cells.length - 1] || null;

  const addCode = document.createElement('button');
  addCode.className = 'nb-add-btn';
  addCode.textContent = '+ Code';
  addCode.addEventListener('click', () => insertCell(last, 'code'));

  const addMd = document.createElement('button');
  addMd.className = 'nb-add-btn';
  addMd.textContent = '+ Markdown';
  addMd.addEventListener('click', () => insertCell(last, 'markdown'));

  row.append(addCode, addMd);
  return row;
}

function renderCell(cell) {
  const el = document.createElement('div');
  el.className = 'nb-cell nb-cell--' + cell.type;

  const gutter = document.createElement('div');
  gutter.className = 'nb-gutter';
  const body = document.createElement('div');
  body.className = 'nb-body';

  if (cell.type === 'code') {
    const label = document.createElement('span');
    label.className = 'nb-gutter__label';
    label.textContent = `In [${cell.execCount ?? ' '}]:`;
    const run = document.createElement('button');
    run.className = 'nb-gutter__run';
    run.title = 'Run this cell';
    run.textContent = '▶';
    run.addEventListener('click', () => runCell(cell));
    gutter.append(run, label);

    body.appendChild(renderCodeInput(cell));
    const outs = document.createElement('div');
    outs.className = 'nb-outputs';
    body.appendChild(outs);
    cell._outsEl = outs;
    renderOutputs(cell);
  } else if (cell.type === 'markdown') {
    const md = document.createElement('div');
    md.className = 'nb-md';
    renderMarkdownInto(md, cell);
    if (state.canEdit) {
      md.classList.add('nb-md--editing');
      md.title = 'Double-click to edit';
      md.addEventListener('dblclick', () => editMarkdown(cell));
    }
    body.appendChild(md);
    cell._mdEl = md;
  } else {
    const raw = document.createElement('pre');
    raw.className = 'nb-raw';
    raw.textContent = cell.source;
    body.appendChild(raw);
  }

  if (state.canEdit) gutter.appendChild(renderCellControls(cell));

  el.append(gutter, body);
  return el;
}

function renderCodeInput(cell) {
  const wrap = document.createElement('div');
  wrap.className = 'nb-input';

  if (state.canEdit) {
    const ta = document.createElement('textarea');
    ta.spellcheck = false;
    ta.value = cell.source;
    ta.rows = Math.max(1, cell.source.split('\n').length);
    const grow = () => { ta.style.height = 'auto'; ta.style.height = ta.scrollHeight + 'px'; };
    ta.addEventListener('input', () => { cell.source = ta.value; grow(); });
    ta.addEventListener('focus', () => wrap.classList.add('nb-input--focus'));
    ta.addEventListener('blur', () => wrap.classList.remove('nb-input--focus'));
    ta.addEventListener('keydown', async (e) => {
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        await runCell(cell);
      } else if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        await runCell(cell);
        const idx = state.nb.cells.indexOf(cell);
        const next = state.nb.cells[idx + 1];
        if (next) focusCell(next);
        else insertCell(cell, 'code');
      } else if (e.key === 'Enter' && e.altKey) {
        e.preventDefault();
        await runCell(cell);
        insertCell(cell, 'code');
      } else if (e.key === 'Tab') {
        e.preventDefault();
        const s = ta.selectionStart;
        ta.value = ta.value.slice(0, s) + '    ' + ta.value.slice(ta.selectionEnd);
        ta.selectionStart = ta.selectionEnd = s + 4;
        cell.source = ta.value;
      }
    });
    wrap.appendChild(ta);
    requestAnimationFrame(grow);
    cell._inputEl = ta;
  } else {
    const pre = document.createElement('pre');
    const code = document.createElement('code');
    const lang = state.nb.language || 'python';
    if (window.hljs && window.hljs.getLanguage(lang)) {
      code.innerHTML = window.hljs.highlight(cell.source, { language: lang, ignoreIllegals: true }).value;
    } else {
      code.textContent = cell.source;
    }
    pre.className = 'nb-code hljs';
    pre.appendChild(code);
    wrap.appendChild(pre);
  }
  return wrap;
}

function currentSource(cell) {
  return cell._inputEl ? cell._inputEl.value : cell.source;
}

function renderMarkdownInto(el, cell) {
  let src = cell.source;
  const { html } = Md.render(src);
  el.innerHTML = html || '<em class="v-empty" style="padding:0">empty markdown cell</em>';
  // attachment: refs (embedded images in the cell)
  for (const img of el.querySelectorAll('img[src^="attachment:"]')) {
    const name = img.getAttribute('src').slice('attachment:'.length);
    const att = cell.attachments && cell.attachments[name];
    if (att) {
      const mime = Object.keys(att)[0];
      img.src = `data:${mime};base64,${joinMaybe(att[mime])}`;
    }
  }
  // relative images -> /sandbox/
  for (const img of el.querySelectorAll('img[src]')) {
    const s = img.getAttribute('src');
    if (/^(https?:|data:|attachment:)/i.test(s)) continue;
    const resolved = resolvePath(state.path, s);
    if (resolved != null) img.src = '/sandbox/' + enc(resolved);
  }
}

function joinMaybe(v) { return Array.isArray(v) ? v.join('') : v; }

function editMarkdown(cell) {
  const el = cell._mdEl;
  const ta = document.createElement('textarea');
  ta.className = 'nb-input';
  ta.style.width = '100%';
  ta.style.minHeight = '120px';
  ta.value = cell.source;
  ta.spellcheck = false;
  const commit = () => {
    cell.source = ta.value;
    const fresh = document.createElement('div');
    fresh.className = 'nb-md nb-md--editing';
    fresh.title = 'Double-click to edit';
    renderMarkdownInto(fresh, cell);
    fresh.addEventListener('dblclick', () => editMarkdown(cell));
    ta.replaceWith(fresh);
    cell._mdEl = fresh;
    markDirty();
  };
  ta.addEventListener('blur', commit);
  ta.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { ta.value = cell.source; commit(); }
    else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) commit();
    else if (e.key === 'Enter' && e.shiftKey) {
      e.preventDefault();
      commit();
      const idx = state.nb.cells.indexOf(cell);
      const next = state.nb.cells[idx + 1];
      if (next) focusCell(next);
      else insertCell(cell, 'code');
    }
  });
  el.replaceWith(ta);
  ta.focus();
}

// ── Outputs ──────────────────────────────────────────────────────────────

function renderOutputs(cell) {
  const host = cell._outsEl;
  if (!host) return;
  host.innerHTML = '';
  for (const out of cell.outputs || []) {
    host.appendChild(renderOutput(out));
  }
}

function renderOutput(out) {
  const el = document.createElement('div');
  el.className = 'nb-out';

  if (out.output_type === 'stream') {
    el.classList.add(out.name === 'stderr' ? 'nb-out--stderr' : 'nb-out--stdout');
    const pre = document.createElement('pre');
    pre.textContent = stripAnsi(out.text);
    el.appendChild(pre);
    return el;
  }
  if (out.output_type === 'error') {
    el.classList.add('nb-out--error');
    const pre = document.createElement('pre');
    const tb = (out.traceback && out.traceback.length)
      ? out.traceback.map(stripAnsi).join('\n')
      : `${out.ename}: ${out.evalue}`;
    pre.textContent = tb;
    el.appendChild(pre);
    return el;
  }

  // execute_result / display_data — pick the richest MIME we can show
  const data = out.data || {};
  if (data['image/png'] || data['image/jpeg']) {
    const mime = data['image/png'] ? 'image/png' : 'image/jpeg';
    const img = document.createElement('img');
    img.src = `data:${mime};base64,${(joinMaybe(data[mime]) || '').replace(/\s/g, '')}`;
    el.appendChild(img);
    return el;
  }
  if (data['image/svg+xml']) {
    const wrap = document.createElement('div');
    wrap.innerHTML = window.DOMPurify.sanitize(joinMaybe(data['image/svg+xml']), { USE_PROFILES: { svg: true, svgFilters: true } });
    el.appendChild(wrap);
    return el;
  }
  if (data['text/html']) {
    const wrap = document.createElement('div');
    wrap.className = 'nb-out__html';
    wrap.innerHTML = window.DOMPurify.sanitize(joinMaybe(data['text/html']));
    el.appendChild(wrap);
    return el;
  }
  if (data['text/markdown']) {
    const wrap = document.createElement('div');
    wrap.className = 'nb-md';
    wrap.innerHTML = Md.render(joinMaybe(data['text/markdown'])).html;
    el.appendChild(wrap);
    return el;
  }
  if (data['application/json'] !== undefined) {
    const pre = document.createElement('pre');
    pre.textContent = JSON.stringify(data['application/json'], null, 2);
    el.appendChild(pre);
    return el;
  }
  const pre = document.createElement('pre');
  pre.textContent = stripAnsi(joinMaybe(data['text/plain']) || '');
  el.appendChild(pre);
  return el;
}

function stripAnsi(s) {
  return String(s == null ? '' : s).replace(/\x1b\[[0-9;]*m/g, '');
}

// ── Execution ────────────────────────────────────────────────────────────

async function runCell(cell) {
  if (cell.type !== 'code' || state.running) return;
  state.running = true;
  syncToolbar();
  cell._el.classList.add('nb-cell--running');
  cell.outputs = [];
  renderOutputs(cell);

  const push = (o) => { cell.outputs.push(o); renderOutputs(cell); autoScroll(cell); };

  try {
    const { execCount, failed } = await kernel.run(currentSource(cell), cell.id, {
      onStream(name, text) {
        const last = cell.outputs[cell.outputs.length - 1];
        if (last && last.output_type === 'stream' && last.name === name) {
          last.text += text;
          renderOutputs(cell);
        } else {
          push({ output_type: 'stream', name, text });
        }
      },
      onDisplay(data) { push({ output_type: 'display_data', data, metadata: {} }); },
      onResult(data) { push({ output_type: 'execute_result', data, metadata: {}, execution_count: kernel.execCount + 1 }); },
      onError(e) {
        push({ output_type: 'error', ename: e.ename, evalue: e.evalue, traceback: e.traceback || [] });
      },
      onPackages(names) {
        const note = document.createElement('div');
        note.className = 'nb-pkg-note';
        note.textContent = 'loaded ' + names.join(', ');
        cell._outsEl.prepend(note);
      },
    });
    cell.execCount = failed ? cell.execCount : execCount;
  } catch (err) {
    push({ output_type: 'error', ename: 'KernelError', evalue: err.message, traceback: [] });
  } finally {
    cell._el.classList.remove('nb-cell--running');
    updateGutter(cell);
    state.running = false;
    syncToolbar();
    markDirty();
    persist();
  }
}

async function runAll() {
  for (const cell of state.nb.cells) {
    if (cell.type === 'code' && currentSource(cell).trim()) {
      await runCell(cell);
      if (kernel.status === 'dead' || kernel.status === 'uninitialized') break;
    }
  }
}

async function doRestart() {
  state.restarting = true;
  syncToolbar();
  try { await kernel.restart(); toast('Kernel restarted'); }
  catch (e) { toast('Restart failed: ' + e.message); }
  finally { state.restarting = false; syncToolbar(); }
}

function doClear() {
  for (const cell of state.nb.cells) {
    if (cell.type === 'code') { cell.outputs = []; cell.execCount = null; renderOutputs(cell); updateGutter(cell); }
  }
  markDirty();
  persist();
}

async function doSave() {
  state.saving = true;
  syncToolbar();
  try {
    const text = serializeNotebook(state.nb, state.nb.cells);
    await Host.saveVersion([{ path: state.path, content: text }]);
    state.fileHash = hashString(text);
    state.dirty = false;
    if (window.lwid) { try { await window.lwid.store.delete(STORE_PREFIX + state.path); } catch (_) { /* ignore */ } }
    toast('Published new version');
  } catch (err) {
    toast('Save failed: ' + err.message);
  } finally {
    state.saving = false;
    syncToolbar();
  }
}

function updateGutter(cell) {
  const label = cell._el.querySelector('.nb-gutter__label');
  if (label) label.textContent = `In [${cell.execCount ?? ' '}]:`;
}

function autoScroll(cell) {
  const r = cell._el.getBoundingClientRect();
  if (r.bottom > window.innerHeight) cell._el.scrollIntoView({ block: 'nearest' });
}

function markDirty() {
  state.dirty = true;
  syncToolbar();
}

// ── Help ─────────────────────────────────────────────────────────────────

function toggleHelp() {
  if ($helpModal) $helpModal.hidden = !$helpModal.hidden;
}

if ($helpModal) {
  $helpModal.querySelector('.nb-help-modal__close')?.addEventListener('click', () => { $helpModal.hidden = true; });
  $helpModal.querySelector('.nb-help-modal__backdrop')?.addEventListener('click', () => { $helpModal.hidden = true; });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$helpModal.hidden) $helpModal.hidden = true;
  });
}

// ── utils ────────────────────────────────────────────────────────────────

function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}
