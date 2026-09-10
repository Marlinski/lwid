/**
 * kernel.js — a thin async wrapper around the Pyodide worker.
 *
 * The heavy lifting (downloading Pyodide, running code) happens off the main
 * thread in pyodide-worker.js so the UI stays responsive and a runaway cell
 * can be killed by terminating the worker.
 */

const WORKER_URL = '/sandbox/__viewer__/pyodide-worker.js';

export class Kernel {
  constructor({ onStatus } = {}) {
    this.worker = null;
    this.ready = null;
    this.status = 'uninitialized'; // uninitialized | loading | idle | busy | dead
    this.execCount = 0;
    this._onStatus = onStatus || (() => {});
    this._run = null; // active run: { cellId, handlers, resolve }
  }

  _setStatus(s) {
    this.status = s;
    this._onStatus(s);
  }

  /** Boot the worker + Pyodide runtime. Idempotent; returns the ready promise. */
  start() {
    if (this.ready) return this.ready;
    this._setStatus('loading');
    this.worker = new Worker(WORKER_URL);
    this.worker.onmessage = (e) => this._onMessage(e.data);
    this.worker.onerror = (e) => {
      const err = new Error(e.message || 'worker crashed');
      if (this._run) { this._run.reject(err); this._run = null; }
      this._setStatus('dead');
    };
    this.ready = new Promise((resolve, reject) => {
      this._readyResolve = resolve;
      this._readyReject = reject;
    });
    this.worker.postMessage({ type: 'init' });
    return this.ready;
  }

  _onMessage(msg) {
    switch (msg.type) {
      case 'ready':
        this._setStatus('idle');
        this._readyResolve();
        break;
      case 'init-error':
        this._setStatus('dead');
        this._readyReject(new Error(msg.error));
        break;
      case 'stream':
        this._run?.handlers.onStream?.(msg.name, msg.text);
        break;
      case 'display':
        this._run?.handlers.onDisplay?.(msg.data, msg.metadata || {});
        break;
      case 'result':
        this._run?.handlers.onResult?.(msg.data);
        break;
      case 'error':
        this._run?.handlers.onError?.(msg);
        break;
      case 'done':
        this.execCount = msg.execCount ?? this.execCount + 1;
        if (this._run) {
          const r = this._run;
          this._run = null;
          this._setStatus('idle');
          r.resolve({ execCount: this.execCount, failed: !!msg.failed });
        }
        break;
      case 'packages':
        this._run?.handlers.onPackages?.(msg.names);
        break;
      default:
        break;
    }
  }

  /**
   * Run a chunk of code. `handlers` = { onStream, onDisplay, onResult, onError, onPackages }.
   * Resolves { execCount, failed } when the cell finishes.
   */
  async run(code, cellId, handlers = {}) {
    await this.start();
    if (this._run) throw new Error('kernel busy');
    this._setStatus('busy');
    return new Promise((resolve, reject) => {
      this._run = { cellId, handlers, resolve, reject };
      this.worker.postMessage({ type: 'run', code, cellId });
    });
  }

  /** Hard-stop the current run (and the whole runtime — Pyodide has no SAB here). */
  interrupt() {
    if (!this.worker) return;
    this.worker.terminate();
    this.worker = null;
    this.ready = null;
    if (this._run) {
      this._run.handlers.onError?.({
        ename: 'KeyboardInterrupt', evalue: 'execution stopped', traceback: [],
      });
      this._run.resolve({ execCount: this.execCount, failed: true });
      this._run = null;
    }
    this.execCount = 0;
    this._setStatus('uninitialized');
  }

  /** Restart: fresh runtime, counters back to zero. */
  async restart() {
    this.interrupt();
    await this.start();
  }
}
