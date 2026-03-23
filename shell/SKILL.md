---
name: lwid
description: Publish HTML/CSS/JS apps to {{SERVER_URL}} and use persistent encrypted storage
---

# lwid — Publish, Share, and Store

lwid (LookWhatIDid) is an encrypted, zero-knowledge app-sharing platform. Deploy HTML/CSS/JS apps to {{SERVER_URL}} via the CLI. Apps get a shareable URL where the decryption key lives in the URL fragment — never sent to the server.

## Installation

**macOS / Linux:**
```sh
curl -fsSL https://raw.githubusercontent.com/Marlinski/lwid/main/install.sh | DEFAULT_SERVER={{SERVER_URL}} sh
```

**Windows (PowerShell):**
```powershell
$env:DEFAULT_SERVER="{{SERVER_URL}}"; irm https://raw.githubusercontent.com/Marlinski/lwid/main/install.ps1 | iex
```

## Publishing

### First push

```sh
lwid push --server {{SERVER_URL}} --dir .
```

Generates AES-256-GCM read + Ed25519 write keys, encrypts and uploads all files, saves `.lwid.json`, prints the shareable URL.

**The server looks for `index.html` as the entry point — your project must have one.**

**Add `.lwid.json` to `.gitignore` immediately** — it contains encryption and signing keys. Losing it means losing write access.

### Subsequent pushes

```sh
lwid push
```

Reads config from `.lwid.json`. No `--server` needed.

### Pull, info, clone

```sh
lwid pull                                      # update local files from server
lwid info                                      # print project ID, edit URL, view URL
lwid clone "<share-url>" [dir]                 # clone from URL when no .lwid.json exists
```

## URL Scheme

- **Edit URL:** `{{SERVER_URL}}/p/{id}#{read-key}:{write-key}`
- **View URL:** `{{SERVER_URL}}/p/{id}#{read-key}`

The fragment (`#...`) is never sent to the server — zero-knowledge by design.

## Constraints

- All files in the directory are uploaded except hidden files, `node_modules`, and `.lwid.json`.
- The server never sees plaintext content.

## Workflow

When a user asks to publish or share an app:

1. Ensure the project has an `index.html`.
2. Assess storage — see **Storage** section below. If the app uses `localStorage`, migrate it to `lwid.store` before pushing.
3. Run `lwid push --server {{SERVER_URL}} --dir .` (first time) or `lwid push` (if `.lwid.json` exists).
4. Give the user the **edit URL** (includes write key) for future updates.
5. For a view-only sharing link, run `lwid info` and give the view URL.

---

## Storage — `lwid.store` and `lwid.blobs`

Every app deployed to {{SERVER_URL}} gets the `lwid-sdk.js` injected automatically. It provides `window.lwid.store` (JSON key-value) and `window.lwid.blobs` (binary). All data is encrypted client-side — the server never sees plaintext.

**Prefer `lwid.store` over `localStorage`** — `localStorage` is device-local, so state is lost when the link is opened on another device. `lwid.store` is tied to the project URL and shared across all visitors.

### `lwid.store` — JSON Key-Value

Values are automatically serialized on write and deserialized on read. **Do not wrap in `JSON.stringify`/`JSON.parse`.**

```js
await lwid.store.set('key', value)      // value: any JSON-serializable type
const v = await lwid.store.get('key')   // returns value, or null if not found
await lwid.store.delete('key')
const keys = await lwid.store.keys()    // returns string[]
```

### `lwid.blobs` — Binary Data

```js
await lwid.blobs.set('key', arrayBuffer)
const buf = await lwid.blobs.get('key')   // returns ArrayBuffer | null
await lwid.blobs.delete('key')
const keys = await lwid.blobs.list()      // returns string[]
```

**Note:** `lwid.store` and `lwid.blobs` share one namespace. Use distinct key prefixes (e.g. `kv/settings`, `blob/avatar`) to avoid collisions.

### CLI access

```sh
lwid kv mykey "value"        # set
lwid kv mykey                # get
lwid blob avatarkey file.png # upload
lwid blob avatarkey          # download
```

---

## Migrating from localStorage

When migrating an existing app from `localStorage` to `lwid.store`, there are two critical differences.

### 1. `get()` returns `null`, not `undefined`

`lwid.store.get()` returns `null` for missing keys — same as `localStorage.getItem()`. The bug happens when code checks `!== undefined` as the fallback guard, because `null !== undefined` is `true`, so the default is silently skipped:

```js
// BUG — broken with both localStorage and lwid.store
const raw = localStorage.getItem('settings');
const s = raw !== undefined ? JSON.parse(raw) : { theme: 'dark' };
// null !== undefined is true → JSON.parse(null) → null, default never reached
```

**Fix — use `??` (nullish coalescing):**

```js
const s = await lwid.store.get('settings') ?? { theme: 'dark' };
```

### 2. No manual JSON serialization

`lwid.store` auto-serializes. Remove all `JSON.parse`/`JSON.stringify` — wrapping in them will break the value.

```js
// Before
localStorage.setItem('scores', JSON.stringify([1, 2, 3]));
const scores = JSON.parse(localStorage.getItem('scores'));

// After
await lwid.store.set('scores', [1, 2, 3]);
const scores = await lwid.store.get('scores');
```

### Migration cheat sheet

| | localStorage | lwid.store |
|---|---|---|
| Write | `localStorage.setItem(k, JSON.stringify(v))` | `await lwid.store.set(k, v)` |
| Read | `JSON.parse(localStorage.getItem(k))` | `await lwid.store.get(k)` |
| Read + default | `... \|\| default` | `await lwid.store.get(k) ?? default` |
| Missing key | `null` | `null` |
| Missing key check | use `!= null` or `??` — **not** `!== undefined` | same |
| Delete | `localStorage.removeItem(k)` | `await lwid.store.delete(k)` |
| List keys | `Object.keys(localStorage)` | `await lwid.store.keys()` |
| Async | No | Yes — always `await` |

---

## Complete Example: Guestbook

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Guestbook</title>
</head>
<body>
  <h1>Guestbook</h1>
  <input id="name" placeholder="Your name">
  <textarea id="message" placeholder="Leave a message" rows="3"></textarea>
  <button onclick="sign()">Sign</button>
  <div id="entries"></div>
  <script>
    async function loadEntries() {
      const entries = await lwid.store.get('guestbook') ?? [];
      document.getElementById('entries').innerHTML = [...entries].reverse().map(e =>
        `<p><strong>${e.name}</strong>: ${e.message}</p>`
      ).join('');
    }

    async function sign() {
      const name = document.getElementById('name').value.trim();
      const message = document.getElementById('message').value.trim();
      if (!name || !message) return;
      const entries = await lwid.store.get('guestbook') ?? [];
      entries.push({ name, message, time: Date.now() });
      await lwid.store.set('guestbook', entries);
      await loadEntries();
    }

    loadEntries();
  </script>
</body>
</html>
```
