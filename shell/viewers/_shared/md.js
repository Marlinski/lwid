/**
 * md.js — shared Markdown rendering for lwid viewers.
 *
 * Wraps markdown-it + highlight.js + DOMPurify (all loaded as globals by the
 * host page) into one sanitized pipeline used by both the docs viewer and the
 * notebook viewer's Markdown cells.
 *
 * Plain script; exposes `window.LwidMd`.
 */
(function () {
  'use strict';
  if (window.LwidMd || !window.markdownit) return;

  function slugify(text) {
    return String(text)
      .toLowerCase()
      .trim()
      .replace(/[^\w\s-]/g, '')
      .replace(/\s+/g, '-')
      .replace(/-+/g, '-');
  }

  const md = window.markdownit({
    html: true,
    linkify: true,
    typographer: true,
    breaks: false,
    highlight(str, lang) {
      if (window.hljs && lang && window.hljs.getLanguage(lang)) {
        try {
          return '<pre class="hljs"><code>' +
            window.hljs.highlight(str, { language: lang, ignoreIllegals: true }).value +
            '</code></pre>';
        } catch (_) { /* fall through */ }
      }
      const esc = md.utils.escapeHtml(str);
      return '<pre class="hljs"><code>' + esc + '</code></pre>';
    },
  });

  // Heading anchors (id = slug of the heading text).
  const defaultHeadingOpen = md.renderer.rules.heading_open
    || ((tokens, idx, options, env, self) => self.renderToken(tokens, idx, options));
  md.renderer.rules.heading_open = (tokens, idx, options, env, self) => {
    const inline = tokens[idx + 1];
    const text = inline && inline.type === 'inline' ? inline.content : '';
    const slug = slugify(text);
    if (slug) {
      const seen = env.__slugs || (env.__slugs = new Map());
      const n = seen.get(slug) || 0;
      seen.set(slug, n + 1);
      tokens[idx].attrSet('id', n ? `${slug}-${n}` : slug);
    }
    return defaultHeadingOpen(tokens, idx, options, env, self);
  };

  // Open external links in a new tab.
  const defaultLinkOpen = md.renderer.rules.link_open
    || ((tokens, idx, options, env, self) => self.renderToken(tokens, idx, options));
  md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
    const href = tokens[idx].attrGet('href') || '';
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(href)) {
      tokens[idx].attrSet('target', '_blank');
      tokens[idx].attrSet('rel', 'noopener noreferrer');
    }
    return defaultLinkOpen(tokens, idx, options, env, self);
  };

  const PURIFY_OPTS = {
    ADD_ATTR: ['target', 'id'],
    // Allow the classes highlight.js and KaTeX emit.
    ADD_TAGS: ['span'],
  };

  function parseFrontmatter(src) {
    const m = src.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
    if (!m) return { body: src, meta: {} };
    const meta = {};
    for (const line of m[1].split(/\r?\n/)) {
      const kv = line.match(/^\s*([A-Za-z0-9_.-]+)\s*:\s*(.*)$/);
      if (kv) meta[kv[1]] = kv[2].trim().replace(/^["']|["']$/g, '');
    }
    return { body: src.slice(m[0].length), meta };
  }

  window.LwidMd = {
    slugify,

    /**
     * Render a Markdown document to sanitized HTML.
     * @param {string} src
     * @returns {{ html: string, meta: Record<string,string>, headings: {level:number,text:string,slug:string}[] }}
     */
    render(src) {
      const { body, meta } = parseFrontmatter(src);
      const env = {};
      const rawHtml = md.render(body, env);
      const html = window.DOMPurify
        ? window.DOMPurify.sanitize(rawHtml, PURIFY_OPTS)
        : rawHtml;

      const headings = [];
      const re = /^(#{1,6})\s+(.+?)\s*#*\s*$/gm;
      let mm;
      const seen = new Map();
      while ((mm = re.exec(body))) {
        const level = mm[1].length;
        const text = mm[2].replace(/[*_`]/g, '');
        let slug = slugify(text);
        const n = seen.get(slug) || 0;
        seen.set(slug, n + 1);
        if (n) slug = `${slug}-${n}`;
        headings.push({ level, text, slug });
      }

      return { html, meta, headings };
    },

    /** Render a single line of Markdown (no block elements). */
    renderInline(src) {
      const raw = md.renderInline(String(src));
      return window.DOMPurify ? window.DOMPurify.sanitize(raw, PURIFY_OPTS) : raw;
    },
  };
})();
