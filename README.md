# lwid

Encrypted, zero-knowledge app-sharing platform. Pastebin for small web apps, with client-side encryption.

**Repo**: https://github.com/Marlinski/lwid

## How it works

Content is encrypted client-side with AES-256-GCM before upload. The server only stores opaque, content-addressed blobs (IPFS CIDv1, SHA2-256) and never sees plaintext. Decryption keys live exclusively in the URL fragment (`#key`), which browsers never send to the server. Write access is authenticated via Ed25519 signatures, so the server can verify authorship without knowing what it's hosting.

## Quick start (CLI)

```sh
curl -fsSL https://raw.githubusercontent.com/Marlinski/lwid/main/install.sh | sh
cd my-project/
lwid push
# Returns: https://lookwhatidid.ovh/p/abc123#readkey:writekey
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

| Command     | Description                                                                 |
|-------------|-----------------------------------------------------------------------------|
| `lwid push` | Encrypt and upload a directory to the server. Returns the project URL.      |
| `lwid pull` | Download and decrypt a project given its URL (including the fragment key).  |
| `lwid info` | Display metadata for a project (size, blob count) without decrypting.      |

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
| `data_dir`      | `LWID_STORAGE__DATA_DIR`      | `./data`          | Directory for blob and project data  |
| `max_blob_size` | `LWID_SERVER__MAX_BLOB_SIZE`  | `10485760` (10MB) | Maximum size of a single blob upload |
| `cors_origins`  | `LWID_SERVER__CORS_ORIGINS`   | `*`               | Allowed CORS origins (comma-separated) |
| `shell_dir`     | `LWID_SERVER__SHELL_DIR`      | `./shell`         | Path to the shell SPA directory      |

## Architecture

Rust workspace with three crates:

```
lwid-common/   Shared types, crypto (AES-256-GCM, Ed25519), CID utilities
lwid-server/   Axum HTTP server, blob storage, shell SPA serving
lwid-cli/      CLI binary (`lwid`), push/pull/info commands
```

The shell SPA is vanilla JS served by the Rust server. It intercepts navigation via a Service Worker and renders decrypted project content inside a sandboxed iframe.

## Building from source

```sh
git clone https://github.com/Marlinski/lwid.git
cd lwid
cargo build --release --workspace
```

Binaries are written to `target/release/`. The server binary is `lwid-server`, the CLI is `lwid`.

## License

MIT
