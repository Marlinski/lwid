/**
 * pyodide-worker.js — runs Python in a Web Worker via Pyodide.
 *
 * Protocol (main <-> worker):
 *   -> { type: 'init' }
 *   <- { type: 'ready' } | { type: 'init-error', error }
 *   -> { type: 'mount', files: [{ path, bytes: ArrayBuffer }] }
 *   <- { type: 'mounted' }
 *   -> { type: 'run', code, cellId }
 *   <- { type: 'packages', names }        (auto-loaded imports, if any)
 *   <- { type: 'stream', cellId, name, text }
 *   <- { type: 'display', cellId, data, metadata }
 *   <- { type: 'result', cellId, data }   (mimebundle for the last expression)
 *   <- { type: 'error', cellId, ename, evalue, traceback }
 *   <- { type: 'done', cellId, execCount, failed? }
 */

const PYODIDE_VERSION = 'v0.27.7';
const INDEX_URL = `https://cdn.jsdelivr.net/pyodide/${PYODIDE_VERSION}/full/`;
const WORKDIR = '/home/pyodide';

let pyodide = null;
let execCount = 0;

// Python-side execution harness: keeps one persistent namespace, splits off a
// trailing expression so it can be echoed like a REPL, formats rich reprs, and
// sweeps matplotlib figures after every cell.
const PREAMBLE = `
import ast, sys, io, base64, json, os

__lwid_ns__ = {'__name__': '__main__'}

# Force a headless backend: matplotlib-pyodide's default backend needs a DOM
# document, which does not exist inside a Web Worker. MPLBACKEND is read when
# matplotlib is first imported, so setting it here is enough.
os.environ['MPLBACKEND'] = 'AGG'

# The project's other files (see mount()) live here, and this is where a cell
# lands by default — so pd.read_csv('data.csv') / open('./data.csv') just work.
os.makedirs('${WORKDIR}', exist_ok=True)
os.chdir('${WORKDIR}')


def __lwid_format__(obj):
    if obj is None:
        return None
    bundle = {}
    for meth, mime in (
        ('_repr_html_', 'text/html'),
        ('_repr_markdown_', 'text/markdown'),
        ('_repr_svg_', 'image/svg+xml'),
        ('_repr_latex_', 'text/latex'),
        ('_repr_json_', 'application/json'),
    ):
        fn = getattr(obj, meth, None)
        if callable(fn):
            try:
                val = fn()
                if val:
                    bundle[mime] = val
            except Exception:
                pass
    fn = getattr(obj, '_repr_png_', None)
    if callable(fn):
        try:
            val = fn()
            if val:
                bundle['image/png'] = base64.b64encode(val).decode() if isinstance(val, (bytes, bytearray)) else val
        except Exception:
            pass
    if not bundle:
        # Suppress the bare repr of matplotlib artists — the figure sweep already
        # shows the plot, and "[<matplotlib.lines.Line2D ...>]" is just noise.
        mod = getattr(type(obj), '__module__', '') or ''
        if mod.startswith('matplotlib') and not mod.startswith('matplotlib.figure'):
            return None
    bundle.setdefault('text/plain', repr(obj))
    return json.dumps(bundle)


def __lwid_run__(src):
    tree = ast.parse(src)
    result = None
    if tree.body and isinstance(tree.body[-1], ast.Expr):
        last = ast.Expression(tree.body.pop().value)
        if tree.body:
            exec(compile(tree, '<cell>', 'exec'), __lwid_ns__)
        result = eval(compile(last, '<cell>', 'eval'), __lwid_ns__)
    else:
        exec(compile(tree, '<cell>', 'exec'), __lwid_ns__)
    return __lwid_format__(result)


def __lwid_figures__():
    out = []
    try:
        import matplotlib.pyplot as plt
        for num in plt.get_fignums():
            fig = plt.figure(num)
            buf = io.BytesIO()
            fig.savefig(buf, format='png', bbox_inches='tight', dpi=96)
            out.append(base64.b64encode(buf.getvalue()).decode())
        plt.close('all')
    except Exception:
        pass
    return out
`;

async function init() {
  importScripts(INDEX_URL + 'pyodide.js');
  // eslint-disable-next-line no-undef
  pyodide = await loadPyodide({
    indexURL: INDEX_URL,
    stdin: () => { throw new Error('input() is not available in the browser kernel'); },
  });
  await pyodide.runPythonAsync(PREAMBLE);
  self.postMessage({ type: 'ready' });
}

/** Write the project's other files into WORKDIR so plain relative-path
 * opens (pd.read_csv('data.csv'), open('./notes.txt')) work like they
 * would in a real notebook folder. Best-effort per file — one unreadable
 * or oddly-pathed file shouldn't stop the kernel from coming up. */
async function mount(files) {
  for (const f of files || []) {
    try {
      const rel = String(f.path || '').replace(/^\/+/, '');
      if (!rel || rel.includes('..')) continue;
      const path = `${WORKDIR}/${rel}`;
      const dir = path.slice(0, path.lastIndexOf('/'));
      if (dir) pyodide.FS.mkdirTree(dir);
      pyodide.FS.writeFile(path, new Uint8Array(f.bytes));
    } catch (_) { /* best effort */ }
  }
  self.postMessage({ type: 'mounted' });
}

async function run(code, cellId) {
  const send = (o) => self.postMessage({ cellId, ...o });

  try {
    // Auto-load imported packages first, with the loader's own chatter routed
    // to a note rather than the cell's stdout.
    try {
      const before = new Set(Object.keys(pyodide.loadedPackages || {}));
      const loaded = await pyodide.loadPackagesFromImports(code, {
        messageCallback: () => {},
        errorCallback: () => {},
      });
      const names = Array.isArray(loaded)
        ? loaded.map((p) => (p && p.name) || p).filter(Boolean)
        : Object.keys(pyodide.loadedPackages || {}).filter((p) => !before.has(p));
      if (names.length) send({ type: 'packages', names });
    } catch (_) { /* import auto-load is best effort */ }

    pyodide.setStdout({ batched: (t) => send({ type: 'stream', name: 'stdout', text: t.endsWith('\n') ? t : t + '\n' }) });
    pyodide.setStderr({ batched: (t) => send({ type: 'stream', name: 'stderr', text: t.endsWith('\n') ? t : t + '\n' }) });

    const bundleJson = await pyodide.runPythonAsync(`__lwid_run__(${JSON.stringify(code)})`);

    // matplotlib figures produced by this cell
    const figsProxy = pyodide.runPython('__lwid_figures__()');
    const figs = figsProxy.toJs();
    figsProxy.destroy();
    for (const b64 of figs) {
      send({ type: 'display', data: { 'image/png': b64 }, metadata: {} });
    }

    if (bundleJson) {
      send({ type: 'result', data: JSON.parse(bundleJson) });
    }
    send({ type: 'done', execCount: ++execCount });
  } catch (err) {
    const text = String(err && err.message ? err.message : err);
    // Pyodide puts the Python traceback in the message; last line is "EName: value".
    const lines = text.split('\n').filter(Boolean);
    const lastLine = lines[lines.length - 1] || 'Error';
    const m = lastLine.match(/^([A-Za-z_.]*(?:Error|Exception|Interrupt|Warning|KeyboardInterrupt))\b:?\s*(.*)$/);
    send({
      type: 'error',
      ename: m ? m[1] : 'PythonError',
      evalue: m ? m[2] : lastLine,
      traceback: lines,
    });
    send({ type: 'done', execCount: ++execCount, failed: true });
  } finally {
    // Restore defaults so the next cell's package auto-load stays quiet.
    try { pyodide.setStdout(); pyodide.setStderr(); } catch (_) { /* ignore */ }
  }
}

self.onmessage = async (e) => {
  const msg = e.data || {};
  try {
    if (msg.type === 'init') {
      await init();
    } else if (msg.type === 'mount') {
      await mount(msg.files);
    } else if (msg.type === 'run') {
      await run(msg.code, msg.cellId);
    }
  } catch (err) {
    self.postMessage({ type: 'init-error', error: String(err && err.message ? err.message : err) });
  }
};
