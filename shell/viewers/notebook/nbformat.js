/**
 * nbformat.js — parse and serialize Jupyter notebooks (.ipynb).
 *
 * Normalises nbformat 4 (and a best-effort pass over the older v3 layout)
 * into a flat cell list the viewer works with, and writes it back out as a
 * spec-shaped nbformat-4 document for "Save .ipynb".
 */

let uid = 0;
const nextId = () => `c${Date.now().toString(36)}${(uid++).toString(36)}`;

/** Join an nbformat "multiline string" (string | string[]) into one string. */
export function joinSource(src) {
  if (Array.isArray(src)) return src.join('');
  return src == null ? '' : String(src);
}

/** Split a string back into the array-of-lines form nbformat prefers. */
function splitSource(str) {
  if (str === '') return [];
  const parts = str.split('\n');
  return parts.map((p, i) => (i < parts.length - 1 ? p + '\n' : p)).filter((p) => p !== '');
}

function normOutput(o) {
  const t = o.output_type;
  if (t === 'stream') {
    return { output_type: 'stream', name: o.name || 'stdout', text: joinSource(o.text) };
  }
  if (t === 'error') {
    return {
      output_type: 'error',
      ename: o.ename || 'Error',
      evalue: o.evalue || '',
      traceback: Array.isArray(o.traceback) ? o.traceback : [],
    };
  }
  if (t === 'execute_result' || t === 'display_data' || t === 'update_display_data') {
    const data = {};
    for (const [mime, val] of Object.entries(o.data || {})) {
      data[mime] = mime === 'application/json' ? val : joinSource(val);
    }
    return {
      output_type: t === 'update_display_data' ? 'display_data' : t,
      data,
      metadata: o.metadata || {},
      ...(o.execution_count != null ? { execution_count: o.execution_count } : {}),
    };
  }
  // v3 fallbacks: pyout / pyerr / stream already handled above
  if (t === 'pyout') {
    return normOutput({ ...o, output_type: 'execute_result' });
  }
  if (t === 'pyerr') {
    return normOutput({ ...o, output_type: 'error' });
  }
  return { output_type: t || 'display_data', data: {}, metadata: {} };
}

function normCell(raw) {
  const type = raw.cell_type === 'heading' ? 'markdown' : (raw.cell_type || 'code');
  const source = joinSource(raw.source ?? raw.input);
  const cell = {
    id: raw.id || nextId(),
    type,
    source,
    metadata: raw.metadata || {},
  };
  if (type === 'code') {
    cell.execCount = raw.execution_count ?? raw.prompt_number ?? null;
    cell.outputs = (raw.outputs || []).map(normOutput);
  }
  if (type === 'markdown' && raw.attachments) {
    cell.attachments = raw.attachments;
  }
  return cell;
}

/**
 * Parse `.ipynb` text into a normalised notebook.
 * @param {string} text
 * @returns {{ cells: object[], language: string, meta: object, nbformat: number }}
 */
export function parseNotebook(text) {
  const nb = JSON.parse(text);

  let rawCells = nb.cells;
  if (!rawCells && Array.isArray(nb.worksheets)) {
    rawCells = nb.worksheets.flatMap((w) => w.cells || []);
  }
  rawCells = rawCells || [];

  const language =
    nb.metadata?.language_info?.name ||
    nb.metadata?.kernelspec?.language ||
    'python';

  return {
    cells: rawCells.map(normCell),
    language: String(language).toLowerCase(),
    meta: nb.metadata || {},
    nbformat: nb.nbformat || 4,
  };
}

/**
 * Serialize a normalised notebook back to nbformat-4 JSON text.
 * @param {{ language: string, meta: object }} nb
 * @param {object[]} cells
 * @returns {string}
 */
export function serializeNotebook(nb, cells) {
  const out = {
    cells: cells.map((c) => {
      const base = {
        cell_type: c.type,
        id: c.id,
        metadata: c.metadata || {},
        source: splitSource(c.source),
      };
      if (c.type === 'markdown' && c.attachments) base.attachments = c.attachments;
      if (c.type !== 'code') return base;
      return {
        ...base,
        execution_count: c.execCount ?? null,
        outputs: (c.outputs || []).map((o) => {
          if (o.output_type === 'stream') return { output_type: 'stream', name: o.name, text: splitSource(o.text) };
          if (o.output_type === 'error') {
            return { output_type: 'error', ename: o.ename, evalue: o.evalue, traceback: o.traceback };
          }
          const data = {};
          for (const [mime, val] of Object.entries(o.data || {})) {
            data[mime] = mime === 'application/json' || typeof val !== 'string' ? val : splitSource(val);
          }
          const rec = { output_type: o.output_type, data, metadata: o.metadata || {} };
          if (o.execution_count != null) rec.execution_count = o.execution_count;
          return rec;
        }),
      };
    }),
    metadata: {
      ...nb.meta,
      language_info: nb.meta?.language_info || { name: nb.language || 'python' },
    },
    nbformat: 4,
    nbformat_minor: 5,
  };
  return JSON.stringify(out, null, 1);
}

export { nextId };
