# LookWhatIDid - Implementation Plan

## Overview

A pastebin for small backendless apps. Users upload HTML/CSS/JS (+ CSV/SQLite),
everything is encrypted client-side, stored as content-addressed blobs (CIDv1),
and shareable via a URL containing the decryption key in the fragment.

## Architecture

```
Browser (Shell SPA)           Rust Server (stateless)        Storage (directory)
 - AES-256-GCM encryption     - blob store (CID -> file)     - blobs/{cid}
 - Ed25519 signing             - project pointers             - projects/{id}.json
 - CID computation             - signature verification
 - Service Worker              - static file serving
 - iframe sandbox
```

## URL Scheme

```
View:  https://lookwhatidid.ovh/p/{project-id}#{read-key}
Edit:  https://lookwhatidid.ovh/p/{project-id}#{read-key}:{write-key}
```

## Phases

### Phase 1: Rust Server Foundation [DONE]
- [x] Project scaffold (Cargo.toml, directory structure)
- [x] Config layer (config.toml -> env -> CLI flags)
- [x] CID module (CIDv1 with sha2-256 multihash, raw codec)
- [x] Blob store trait + filesystem implementation
- [x] Unit tests for CID and blob store

### Phase 2: Rust Server API [DONE]
- [x] Error types
- [x] Project management (create, read, update root)
- [x] Ed25519 auth module
- [x] API routes: blobs (GET, POST, HEAD, batch)
- [x] API routes: projects (POST, GET, PUT root)
- [x] Shell SPA static serving at /p/{id}
- [x] CORS middleware
- [x] Integration tests

### Phase 3: Shell SPA - Crypto & Core [DONE]
- [x] crypto.js (AES-256-GCM encrypt/decrypt, Ed25519 keygen/sign)
- [x] cid.js (CIDv1 computation matching server)
- [x] api.js (HTTP client for server endpoints)
- [x] manifest.js (manifest creation, parsing, chain traversal)

### Phase 4: Shell SPA - Project Flows [DONE]
- [x] project.js (create, push, pull, version history)
- [x] Upload flow (encrypt files, compute CIDs, deduplicate, push)
- [x] Download flow (fetch manifest, resolve CIDs, decrypt)
- [x] Version chain traversal

### Phase 5: Shell SPA - Rendering [DONE]
- [x] Service Worker (intercept fetch, serve decrypted files from in-memory cache)
- [x] MessageChannel acknowledgment (SW confirms files cached before iframe loads)
- [x] Sandboxed iframe renderer via SW fetch interception (/sandbox/ prefix)
- [x] First-load handling (one-time reload if SW not yet controlling page)
- [ ] sql.js integration (postMessage bridge) — deferred

### Phase 6: Shell SPA - UI [DONE]
- [x] index.html (shell chrome)
- [x] Upload UI (drag & drop, file picker)
- [x] Version history panel
- [x] Share buttons (read-only / read-write URLs)
- [x] Download menu (individual file download)
- [x] Responsive styling

### Phase 7: Static Serving & Polish [IN PROGRESS]
- [x] Static file serving from Rust server (shell_dir config, ServeDir/ServeFile)
- [x] SPA fallback: `/p/{id}` → index.html
- [x] Smoke test: start server, verify static serving + API together
- [x] End-to-end integration test (create → push → pull → history)
- [ ] SKILL.md for AI-assisted vibe-coding
- [ ] Dockerfile
- [ ] Documentation

## Key Interfaces

### Rust: BlobStore trait
```rust
trait BlobStore {
    fn put(&self, data: &[u8]) -> Result<Cid>;
    fn get(&self, cid: &Cid) -> Result<Vec<u8>>;
    fn exists(&self, cid: &Cid) -> Result<bool>;
}
```

### Rust: ProjectStore trait
```rust
trait ProjectStore {
    fn create(&self, id: &str, write_pubkey: &[u8]) -> Result<Project>;
    fn get(&self, id: &str) -> Result<Project>;
    fn update_root(&self, id: &str, root_cid: &Cid) -> Result<()>;
}
```

### JS: Manifest schema (encrypted)
```json
{
  "version": 5,
  "parent_cid": "bafyPreviousRoot...",
  "timestamp": "2026-03-19T14:30:00Z",
  "files": [
    {"path": "index.html", "cid": "bafyBlob1...", "size": 2048}
  ]
}
```

## Config (config.toml)
```toml
[storage]
data_dir = "/data"

[server]
listen = "0.0.0.0:8080"
max_blob_size = 10_485_760   # 10MB
max_project_size = 10_485_760
```
