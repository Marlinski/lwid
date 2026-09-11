/**
 * viewer-ui.js — tiny shared UI helpers for viewer bundles.
 * Plain script; exposes `window.LwidUI`.
 */
(function () {
  'use strict';
  if (window.LwidUI) return;

  let toastEl = null;
  let toastTimer = null;

  function toast(message, ms = 2400) {
    if (!toastEl) {
      toastEl = document.createElement('div');
      toastEl.className = 'v-toast';
      document.body.appendChild(toastEl);
    }
    toastEl.textContent = message;
    // reflow so the transition re-triggers
    void toastEl.offsetWidth;
    toastEl.classList.add('v-toast--show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove('v-toast--show'), ms);
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => (
      { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
    ));
  }

  function formatBytes(n) {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  /** Resolve `ref` (a relative href/src) against the directory of `fromPath`. */
  function resolvePath(fromPath, ref) {
    if (/^([a-z]+:)?\/\//i.test(ref) || ref.startsWith('data:') || ref.startsWith('#')) return null;
    const baseParts = fromPath.split('/').slice(0, -1);
    const refParts = ref.replace(/^\.\//, '').split('/');
    for (const part of refParts) {
      if (part === '..') baseParts.pop();
      else if (part !== '.' && part !== '') baseParts.push(part);
    }
    return baseParts.join('/');
  }

  // ── Manual light/dark override ──────────────────────────────────────────
  // Opt-in per viewer (call `theme.apply()` at boot + wire a toolbar button
  // to `theme.toggle()`) — viewers that never touch this keep following
  // prefers-color-scheme only, per viewer.css's default rules.
  const THEME_KEY = 'lwid:viewer-theme';

  function themeStored() {
    try { return localStorage.getItem(THEME_KEY); } catch (_) { return null; }
  }
  function themeStore(v) {
    try { if (v) localStorage.setItem(THEME_KEY, v); else localStorage.removeItem(THEME_KEY); } catch (_) { /* ignore */ }
  }
  function themeEffective() {
    return themeStored() ||
      (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  }
  function themeApply() {
    const t = themeEffective();
    document.documentElement.dataset.theme = t;
    // highlight.js theme stylesheets normally follow prefers-color-scheme
    // via their own `media` attribute, which doesn't know about a manual
    // override — flip them explicitly so code blocks match.
    const light = document.getElementById('hljs-light');
    const dark = document.getElementById('hljs-dark');
    if (light) light.disabled = t !== 'light';
    if (dark) dark.disabled = t !== 'dark';
  }
  function themeToggle() {
    themeStore(themeEffective() === 'dark' ? 'light' : 'dark');
    themeApply();
  }
  function themeToolbarItem() {
    const dark = themeEffective() === 'dark';
    return {
      kind: 'button', id: 'theme',
      label: dark ? '☀️' : '🌙',
      title: dark ? 'Switch to light theme' : 'Switch to dark theme',
    };
  }

  window.LwidUI = {
    toast, escapeHtml, formatBytes, resolvePath,
    theme: { apply: themeApply, toggle: themeToggle, effective: themeEffective, toolbarItem: themeToolbarItem },
  };
})();
