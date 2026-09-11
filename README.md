# lwid

Encrypted, zero-knowledge app-sharing platform. Pastebin for small web apps, with client-side encryption.

**Ship your app in seconds — give your AI agent the [lwid skill](https://lookwhatidid.xyz/SKILL.md) and let it handle the rest.**

![lookwhatidid](screenshot.png?raw=true)

**Live**: https://lookwhatidid.xyz  
**Repo**: https://github.com/Marlinski/lwid

## How it works

Content is encrypted client-side with AES-256-GCM before upload. The server only stores opaque, content-addressed blobs (IPFS CIDv1, SHA2-256) and never sees plaintext. Decryption keys live exclusively in the URL fragment (`#key`), which browsers never send to the server. Write access is authenticated via Ed25519 signatures, so the server can verify authorship without knowing what it's hosting.

## Quick start (CLI)

**macOS / Linux:**
```sh
curl -fsSL https://raw.githubusercontent.com/Marlinski/lwid/main/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/Marlinski/lwid/main/install.ps1 | iex
```

Then push a project:
```sh
cd my-project/
lwid push --server https://lookwhatidid.xyz
# Returns: https://lookwhatidid.xyz/p/abc123#readkey:writekey
```

## Quick start (Server)

### Docker

```sh
docker run -p 8080:8080 -v lwid-data:/data ghcr.io/marlinski/lwid-server
```

### Cargo

```sh
cargo run --release -p lwid-server
```

The server listens on `0.0.0.0:8080` by default and serves the shell SPA, which renders uploaded apps in a sandboxed iframe via Service Worker.

## CLI reference

| Command          | Description                                                                      |
|------------------|----------------------------------------------------------------------------------|
| `lwid push`      | Encrypt and upload a directory to the server. Returns the project URL.           |
| `lwid pull`      | Download and decrypt a project into the current directory (requires `.lwid.json`). |
| `lwid clone <url>` | Clone a project from a share URL into a new directory.                         |
| `lwid info`      | Display project ID, server, edit URL, and view-only URL.                         |
| `lwid kv`        | Get or set a persistent encrypted key-value pair on the project store.           |
| `lwid blob`      | Get or set a persistent encrypted binary blob on the project store.              |
| `lwid login`     | Authenticate with the server via browser (OAuth / magic link).                   |
| `lwid logout`    | Remove the saved authentication token.                                           |

Project config is saved to `.lwid.json` in the project directory — add it to `.gitignore` immediately, it contains your encryption and signing keys.

## URL scheme

```
View:  /p/{project-id}#{read-key}
Edit:  /p/{project-id}#{read-key}:{write-key}
```

Keys are base64url-encoded. The fragment is never sent to the server.

## Server configuration

Configuration is resolved in order of priority: **CLI flags > environment variables > `config.toml` > defaults**.

Environment variables use the `LWID_` prefix.

| Option          | Env var                       | Default          | Description                          |
|-----------------|-------------------------------|------------------|--------------------------------------|
| `listen`        | `LWID_SERVER__LISTEN`         | `0.0.0.0:8080`   | Address and port to bind             |
| `backend`       | `LWID_STORAGE__BACKEND`       | `fs`              | Storage backend: `fs` or `s3`        |
| `data_dir`      | `LWID_STORAGE__DATA_DIR`      | `./data`          | Directory for blob and project data (`fs` backend) |
| `max_blob_size` | `LWID_SERVER__MAX_BLOB_SIZE`  | `10485760` (10MB) | Maximum size of a single blob upload |
| `cors_origins`  | `LWID_SERVER__CORS_ORIGINS`   | `*`               | Allowed CORS origins (comma-separated) |
| `shell_dir`     | `LWID_SERVER__SHELL_DIR`      | `./shell`         | Path to the shell SPA directory      |

### S3 backend

The server can store everything directly in an S3-compatible bucket instead of
on disk — no mounted filesystem, no metadata service, no persistent volume.
The filesystem is used unless `backend` explicitly says `s3`.

```toml
[storage]
backend = "s3"

[storage.s3]
endpoint = "https://s3.gra.io.cloud.ovh.net"
region   = "gra"
bucket   = "lwid"
prefix   = ""      # optional key prefix
```

| Option              | Env var                                | Default | Description                        |
|---------------------|----------------------------------------|---------|------------------------------------|
| `endpoint`          | `LWID_STORAGE__S3__ENDPOINT`           | —       | S3 endpoint URL (required)         |
| `region`            | `LWID_STORAGE__S3__REGION`             | —       | Region name (required)             |
| `bucket`            | `LWID_STORAGE__S3__BUCKET`             | —       | Bucket name (required)             |
| `prefix`            | `LWID_STORAGE__S3__PREFIX`             | none    | Key prefix inside the bucket       |
| `access_key_id`     | `LWID_STORAGE__S3__ACCESS_KEY_ID`      | —       | Falls back to `AWS_ACCESS_KEY_ID`  |
| `secret_access_key` | `LWID_STORAGE__S3__SECRET_ACCESS_KEY`  | —       | Falls back to `AWS_SECRET_ACCESS_KEY` |
| `force_path_style`  | `LWID_STORAGE__S3__FORCE_PATH_STYLE`   | `true`  | Path-style addressing              |

The bucket layout mirrors the on-disk layout exactly, so a `data_dir` can be
copied into a bucket verbatim — and back:

```
{prefix}blobs/ab/cd/<cid>       content-addressed blobs (immutable)
{prefix}projects/<id>.json      project metadata
{prefix}store/<id>/<key>        per-project KV store
```

**Migrating an existing deployment.** Copy the data directory into the bucket
while the old storage is still mounted, then flip the backend:

```sh
aws s3 sync /path/to/data s3://lwid/ --endpoint-url https://s3.gra.io.cloud.ovh.net
```

If the current storage is JuiceFS, note that its bucket does *not* hold your
files as plain objects — it chunks them, with the metadata living in Redis. You
must copy out through a live JuiceFS mount, and the Redis instance holding that
metadata must be intact when you do.

**Authentication requires a filesystem.** SQLite (users, sessions, project
ownership) cannot live on S3. With no auth provider configured the server runs
fully stateless and needs no volume at all; enabling auth means keeping a small
volume for `lwid.db`.

Building without the S3 backend (drops the AWS SDK dependency):

```sh
cargo build --release -p lwid-server --no-default-features
```

## Architecture

Rust workspace with three crates:

```
lwid-common/   Shared types, crypto (AES-256-GCM, Ed25519), CID utilities
lwid-server/   Axum HTTP server, blob storage, shell SPA serving
lwid-cli/      CLI binary (`lwid`), push/pull/clone/store commands
shell/         Vanilla-JS SPA + Service Worker
shell/viewers/ Per-file-type viewers (notebook, docs, file browser)
```

The shell SPA is vanilla JS served by the Rust server. It intercepts navigation via a Service Worker and renders decrypted project content inside a sandboxed iframe.

### Viewers

A project without an `index.html` is opened through a **viewer** — a small
front-end shim picked from the file types (`shell/js/viewers.js`):

| Files | Viewer |
|-------|--------|
| any `.html` | none — the site renders as-is |
| `*.ipynb` | notebook viewer — renders cells + saved outputs, and runs Python in-browser via a Pyodide Web Worker |
| `*.md` | docs viewer — sidebar (honours `SUMMARY.md`), relative links, per-page TOC |
| anything else | a browsable file listing |

Viewer bundles live in `shell/viewers/` and are served to the sandbox by the
Service Worker under reserved `/sandbox/__viewer__/` and `/sandbox/__shared__/`
prefixes; the project's own files stay at their real paths. Editable links can
publish a new version straight from the viewer. Notebook run-state is kept in
the encrypted project store, so a shared link shows the last execution.

Viewers pull a few libraries from CDN at runtime (markdown-it, highlight.js,
DOMPurify from cdnjs/jsDelivr; Pyodide from jsDelivr) — the same CDN reliance
the shell already has for syntax highlighting. The shell itself normally
needs nothing beyond the browser's Web Crypto API, but on a browser that
doesn't yet recognize Ed25519 there, project creation/signing falls back to
`@noble/ed25519` from jsDelivr, loaded only on browsers that actually need it.

## Building from source

```sh
git clone https://github.com/Marlinski/lwid.git
cd lwid
cargo build --release --workspace
```

Binaries are written to `target/release/`. The server binary is `lwid-server`, the CLI is `lwid`.

## License

MIT

