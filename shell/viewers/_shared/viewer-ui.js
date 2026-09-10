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

  window.LwidUI = { toast, escapeHtml, formatBytes, resolvePath };
})();
