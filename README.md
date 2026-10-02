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
| `lwid push`      | Encrypt and upload a directory to the server. Returns the project URL. `--name` sets the display name. |
| `lwid pull`      | Download and decrypt a project into the current directory (requires `.lwid.json`). |
| `lwid clone <url>` | Clone a project from a share URL into a new directory.                         |
| `lwid info`      | Display project ID, server, edit URL, and view-only URL.                         |
| `lwid kv`        | Get or set a persistent encrypted key-value pair on the project store.           |
| `lwid blob`      | Get or set a persistent encrypted binary blob on the project store.              |
| `lwid login`     | Authenticate with the server via browser (OAuth / magic link).                   |
| `lwid logout`    | Remove the saved authentication token.                                           |

Project config is saved to `.lwid.json` in the project directory — add it to `.gitignore` immediately, it contains your encryption and signing keys.

## Project names

A project can carry a human-readable name, so the projects dropdown shows
"Admin Console" rather than `LDVWqY9uDWSt`. The name is AES-256-GCM encrypted
with the read key and stored in the manifest, exactly like file paths — the
server only ever sees ciphertext.

It is set in the manifest, not the KV store, because the store token is derived
from the *read* key: anyone holding a view-only link can write to the store, and
a name that any visitor could rewrite would be worth little. A manifest is
published under the write key's signature, so renaming is an owner action by
construction.

```sh
lwid push --name "Admin Console"
```

With no `--name`, a new project takes the `<title>` of its entry page, falling
back to the directory name. Later pushes carry the existing name forward. In
the browser, click the project name in the toolbar to rename it (edit links
only); that publishes a new version containing the same files and the new name,
so it re-uploads nothing.

## URL scheme

```
View:  /p/{project-id}#{read-key}
Edit:  /p/{project-id}#{read-key}:{write-key}
```

Keys are base64url-encoded. The fragment is never sent to the server.

## Pushing without the CLI

Nothing about the client side needs the `lwid` binary — it is all standard
primitives over plain HTTP. [`scripts/push-with-openssl.sh`](scripts/push-with-openssl.sh)
is a complete push in about a hundred lines of `bash`, using only `curl`,
`openssl` and coreutils:

```sh
scripts/push-with-openssl.sh -s https://lookwhatidid.xyz README.md data.csv
# https://lookwhatidid.xyz/p/49YwbOFqaQuh#il5eA8Pi...:UFQQhUe_...
```

It doubles as executable documentation of the protocol — encryption, path
encryption, CID derivation, the manifest, and the Ed25519 signature that
publishes a version, in the order they happen. Projects it creates are
readable by `lwid clone` and by the browser, which is how it is tested.

One wrinkle worth knowing if you write your own client: `openssl enc`
refuses AEAD ciphers outright (`enc: AEAD ciphers not supported`), so
AES-256-GCM has to be assembled from `-aes-256-ctr` for the ciphertext plus
`openssl mac ... GMAC` for the tag, with a single GF(2¹²⁸) multiply to
correct GMAC's length block into GCM's. The script explains the derivation.

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

## Metrics

A Prometheus exporter is available at `GET /metrics` on its **own listener**,
off by default:

```toml
[metrics]
enabled = true
listen  = "0.0.0.0:9100"
```

It is deliberately never mounted on `server.listen`. These numbers describe the
whole deployment — user counts, storage totals — and an origin that also serves
untrusted project content is the wrong place for them. Bind it to loopback or
to the pod IP, and keep it out of your Ingress.

| Metric | Labels | Meaning |
|--------|--------|---------|
| `lwid_build_info` | `version` | Always 1; carries the running version |
| `lwid_users_total` | `provider`, `tier` | Registered users |
| `lwid_sessions_active` | `kind` | Unexpired sessions |
| `lwid_signed_in_users` | — | Distinct users with an unexpired session |
| `lwid_project_owners_total` | — | Projects claimed by a user |
| `lwid_projects_total` | — | All projects, live and expired |
| `lwid_projects_with_content` | — | Projects with a published version |
| `lwid_projects_expiring` / `_expired` | — | Future deadline / past it, awaiting the reaper |
| `lwid_project_blob_refs` | — | Blob references across all projects |
| `lwid_projects_by_version` | `created_with` | Projects by creating client version |
| `lwid_blobs_total` | — | Blobs in the content-addressed store |
| `lwid_storage_bytes` | — | Bytes those blobs occupy |
| `lwid_project_detail_enabled` | — | 1 when the per-project breakdown is being collected |
| `lwid_metrics_scrape_duration_seconds` | — | Cost of the last uncached refresh |

Anonymous (unclaimed) projects are `lwid_projects_total -
lwid_project_owners_total`; there is no separate metric for it.

The auth metrics are absent entirely when no auth provider is configured —
there is no database then, and exporting zeros would assert something false.

**Cost.** Every scrape is answered from a 60-second memoised snapshot, so a
tight scrape interval cannot amplify into a storm of storage calls. Behind that
cache one refresh is four grouped `COUNT`s, one `ProjectStore::list()`, and one
`BlobStore::usage()` (a single paginated `ListObjectsV2` on S3). The
per-project breakdown is the one group that costs a read *per project*, which
is why it has its own switch and a `project_detail_limit` ceiling that disables
it rather than letting it grow without bound.

HTTP request rate, latency and status are intentionally **not** exported — the
ingress already produces those per host, and a second in-process source would
only ever disagree with it.

## Architecture

Rust workspace with three crates:

```
lwid-common/   Shared types, crypto (AES-256-GCM, Ed25519), CID utilities
lwid-server/   Axum HTTP server, blob storage, shell SPA serving
lwid-cli/      CLI binary (`lwid`), push/pull/clone/store commands
shell/         Vanilla-JS SPA + Service Worker
shell/viewers/ Per-file-type viewers (notebook, docs, file browser)
```

The shell SPA is vanilla JS served by the Rust server. It decrypts a project
in the page, then hands the files to a Service Worker that serves them to a
sandboxed iframe, so the project behaves like an ordinary static site.

When `server.sandbox_base_domain` is configured, that iframe is served from a
**per-project origin** (`<base32(project-id)>.<domain>`) rather than
same-origin with the shell. This is what keeps one project's scripts away
from the shell's `localStorage` — where other projects' write keys live —
since a sandboxed iframe with `allow-scripts allow-same-origin` can reach
anything on its own origin. The project URL is unaffected: the shell stays on
`/p/{id}#key` and redirects only the iframe, so deployments without wildcard
DNS/TLS simply leave it unset and serve the sandbox same-origin as before.

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
Service Worker under reserved `/__viewer__/` and `/__shared__/` prefixes; the
project's own files stay at their real paths. Editable links can
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

