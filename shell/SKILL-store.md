---
name: lwid-store
description: Build stateful web apps with persistent encrypted storage on lookwhatidid.ovh
---

# lwid Store — Persistent Storage for Web Apps

Build web apps with persistent state on lookwhatidid.ovh. The lwid platform automatically injects a storage SDK into every deployed app, providing encrypted key-value and blob storage that persists across page loads and is shared with all visitors.

## How It Works

When you deploy an HTML app to lookwhatidid.ovh, a small SDK (`lwid-sdk.js`) is automatically injected into every HTML page. This SDK provides `window.lwid.store` (JSON) and `window.lwid.blobs` (binary) APIs. All data is encrypted client-side with AES-256-GCM — the server never sees plaintext.

**No setup required.** Just use the `lwid.store` and `lwid.blobs` APIs in your JavaScript. The SDK is available immediately when your script runs.

## API Reference

### `lwid.store` — JSON Key-Value Store

Values are automatically JSON-serialized on write and deserialized on read.

#### `await lwid.store.get(key)`
- **key** `string` — The key to look up
- **Returns** `any | null` — The deserialized value, or `null` if not found
```js
const score = await lwid.store.get('highscore');
// score is null if never set, or the stored value (number, object, array, etc.)
```

#### `await lwid.store.set(key, value)`
- **key** `string` — The key to store under
- **value** `any` — Any JSON-serializable value (string, number, boolean, object, array)
```js
await lwid.store.set('highscore', 9001);
await lwid.store.set('player', { name: 'Alice', level: 5 });
await lwid.store.set('tags', ['fun', 'game', 'multiplayer']);
```

#### `await lwid.store.delete(key)`
- **key** `string` — The key to delete
```js
await lwid.store.delete('highscore');
```

#### `await lwid.store.keys()`
- **Returns** `string[]` — Array of all stored key names
```js
const allKeys = await lwid.store.keys();
// ['highscore', 'player', 'tags']
```

### `lwid.blobs` — Binary Blob Store

Values are raw `ArrayBuffer`s. Use this for images, files, audio, or any binary data.

#### `await lwid.blobs.get(key)`
- **key** `string` — The key to look up
- **Returns** `ArrayBuffer | null` — The raw bytes, or `null` if not found
```js
const buf = await lwid.blobs.get('avatar');
if (buf) {
  const url = URL.createObjectURL(new Blob([buf], { type: 'image/png' }));
  document.getElementById('avatar').src = url;
}
```

#### `await lwid.blobs.set(key, value)`
- **key** `string` — The key to store under
- **value** `ArrayBuffer` — Raw binary data
```js
// From a fetch response
const resp = await fetch('photo.png');
await lwid.blobs.set('photo', await resp.arrayBuffer());

// From a canvas
const canvas = document.getElementById('canvas');
const blob = await new Promise(r => canvas.toBlob(r, 'image/png'));
await lwid.blobs.set('drawing', await blob.arrayBuffer());

// From a file input
const file = document.getElementById('file-input').files[0];
await lwid.blobs.set('upload', await file.arrayBuffer());
```

#### `await lwid.blobs.delete(key)`
- **key** `string` — The key to delete
```js
await lwid.blobs.delete('avatar');
```

#### `await lwid.blobs.list()`
- **Returns** `string[]` — Array of all stored blob key names
```js
const allBlobs = await lwid.blobs.list();
```

## Limits

- **10 MB** maximum per value (KV or blob)
- **50 MB** total storage per project
- **10 second** timeout per request

## Important Notes

- **Shared state**: All visitors with the project link share the same store. This is ideal for leaderboards, guest books, collaborative data, but means there's no per-user isolation.
- **No authentication**: Anyone with the link can read and write. Design accordingly.
- **Encryption is automatic**: You never handle encryption. Just pass plain values.
- **Keys are strings**: Use descriptive names like `'highscore'`, `'settings'`, `'user/avatar'`.
- **Store and blobs share one namespace**: `lwid.store.set('foo', ...)` and `lwid.blobs.set('foo', ...)` write to the same key. Use distinct key names or prefix them (e.g. `'kv/settings'`, `'blob/avatar'`).
- **Async only**: All methods return Promises. Always `await` them.
- **Available immediately**: The SDK is injected before your scripts run. No need to wait for a load event.

## Deployment

Use the lwid CLI to deploy. See the [lwid SKILL](https://lookwhatidid.ovh/SKILL.md) for full deployment instructions.

Quick reference:
```sh
# First deploy
lwid push --server https://lookwhatidid.ovh --dir . -y

# Update
lwid push
```

You can also manage store data from the CLI:
```sh
# Set a KV value
lwid kv mykey "hello world"

# Get a KV value
lwid kv mykey

# Upload a blob
lwid blob avatar photo.png

# Download a blob
lwid blob avatar > photo.png

# Pipe a blob
cat photo.png | lwid blob avatar
```

## Complete Example: Guestbook App

Here's a minimal but complete app that uses the store:

### index.html
```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Guestbook</title>
  <style>
    body { font-family: system-ui; max-width: 600px; margin: 2rem auto; padding: 0 1rem; }
    .entry { padding: 0.75rem; border: 1px solid #ddd; border-radius: 8px; margin: 0.5rem 0; }
    .entry .name { font-weight: bold; }
    .entry .time { color: #999; font-size: 0.8rem; }
    input, textarea { width: 100%; padding: 0.5rem; margin: 0.25rem 0; box-sizing: border-box; }
    button { padding: 0.5rem 1.5rem; cursor: pointer; }
  </style>
</head>
<body>
  <h1>Guestbook</h1>
  <div>
    <input id="name" placeholder="Your name">
    <textarea id="message" placeholder="Leave a message" rows="3"></textarea>
    <button onclick="sign()">Sign Guestbook</button>
  </div>
  <div id="entries"></div>
  <script>
    async function loadEntries() {
      const entries = await lwid.store.get('guestbook') || [];
      const el = document.getElementById('entries');
      el.innerHTML = entries.map(e => `
        <div class="entry">
          <div class="name">${e.name}</div>
          <div>${e.message}</div>
          <div class="time">${new Date(e.time).toLocaleString()}</div>
        </div>
      `).reverse().join('');
    }

    async function sign() {
      const name = document.getElementById('name').value.trim();
      const message = document.getElementById('message').value.trim();
      if (!name || !message) return alert('Please fill in both fields');

      const entries = await lwid.store.get('guestbook') || [];
      entries.push({ name, message, time: Date.now() });
      await lwid.store.set('guestbook', entries);

      document.getElementById('name').value = '';
      document.getElementById('message').value = '';
      await loadEntries();
    }

    loadEntries();
  </script>
</body>
</html>
```

Deploy with:
```sh
lwid push --server https://lookwhatidid.ovh --dir . -y
```

## Complete Example: Image Gallery

An app that lets visitors upload and view images:

### index.html
```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Gallery</title>
  <style>
    body { font-family: system-ui; max-width: 800px; margin: 2rem auto; padding: 0 1rem; }
    .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 1rem; }
    .gallery img { width: 100%; border-radius: 8px; cursor: pointer; }
    input[type="file"] { margin: 1rem 0; }
  </style>
</head>
<body>
  <h1>Gallery</h1>
  <input type="file" id="upload" accept="image/*" multiple>
  <div class="gallery" id="gallery"></div>
  <script>
    const gallery = document.getElementById('gallery');

    async function loadGallery() {
      const manifest = await lwid.store.get('gallery-manifest') || [];
      gallery.innerHTML = '';
      for (const item of manifest) {
        const buf = await lwid.blobs.get(item.blobKey);
        if (!buf) continue;
        const url = URL.createObjectURL(new Blob([buf], { type: item.type }));
        const img = document.createElement('img');
        img.src = url;
        img.alt = item.name;
        gallery.appendChild(img);
      }
    }

    document.getElementById('upload').addEventListener('change', async (e) => {
      const manifest = await lwid.store.get('gallery-manifest') || [];
      for (const file of e.target.files) {
        const blobKey = 'img/' + Date.now() + '-' + file.name;
        await lwid.blobs.set(blobKey, await file.arrayBuffer());
        manifest.push({ blobKey, name: file.name, type: file.type });
      }
      await lwid.store.set('gallery-manifest', manifest);
      await loadGallery();
    });

    loadGallery();
  </script>
</body>
</html>
```

## Workflow for Agents

When a user asks you to build a stateful web app:

1. Create the HTML/CSS/JS files as a normal static web app
2. Use `lwid.store.get/set/delete/keys` for JSON data (settings, scores, lists, etc.)
3. Use `lwid.blobs.get/set/delete/list` for binary data (images, files, audio, etc.)
4. Deploy with `lwid push --server https://lookwhatidid.ovh --dir . -y`
5. Share the URL with the user

No server code, no database setup, no environment variables — just HTML and JavaScript.
