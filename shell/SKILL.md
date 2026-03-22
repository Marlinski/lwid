---
name: lwid
description: Publish HTML/CSS/JS apps to {{SERVER_URL}} using the lwid CLI
---

# lwid — Publish and Share Web Apps

lwid (LookWhatIDid) is an encrypted, zero-knowledge app-sharing platform. Think pastebin for small web apps. Files are encrypted client-side with AES-256-GCM, stored as content-addressed blobs, and shared via URLs where the decryption key lives in the URL fragment (never sent to the server).

## Installation

```sh
curl -fsSL https://raw.githubusercontent.com/Marlinski/lwid/main/install.sh | sh
```

## Publishing

### First push (new project)

```sh
lwid push --server {{SERVER_URL}} --dir .
```

This generates an AES-256-GCM read key and an Ed25519 write key, creates a project on the server, encrypts and uploads all files, creates a signed manifest, saves `.lwid.json` to the project directory, and prints the shareable URL.

**Add `.lwid.json` to `.gitignore` immediately** — it contains the encryption and signing keys.

### Subsequent pushes

```sh
lwid push
```

Reads server and config from `.lwid.json` in the current directory — no `--server` needed.

### Project info

```sh
lwid info
```

Prints project ID, server, edit URL, and view-only URL.

### Pulling the latest version

```sh
lwid pull
```

Updates local files from the server. Reads server, project ID, and keys from `.lwid.json` in the current directory.

### Cloning a project from a URL

```sh
lwid clone <share-url> [dir]
```

Use this when there is no `.lwid.json` yet. Parses the share URL, downloads and decrypts all files into `dir` (default: current directory), and saves `.lwid.json` so subsequent `lwid push` / `lwid pull` work normally.

```sh
# Clone into a new directory
lwid clone "{{SERVER_URL}}/p/{id}#{read_key}:{write_key}" my-project
# Clone into the current directory
lwid clone "{{SERVER_URL}}/p/{id}#{read_key}"
```

## URL Scheme

- **Edit URL:** `{{SERVER_URL}}/p/{project-id}#{read-key}:{write-key}`
- **View URL:** `{{SERVER_URL}}/p/{project-id}#{read-key}`

The fragment (`#...`) is never sent to the server — zero-knowledge by design.

## .lwid.json

Created automatically by `lwid push`. Contains server URL, project ID, read key, and write key. If this file is lost, the project cannot be updated. Never commit it to version control.

## Workflow

When a user asks to publish or share their app:

1. Verify the project directory contains an `index.html` (the entry point).
2. Run `lwid push --server {{SERVER_URL}} --dir .` from the project directory (first time) or `lwid push` (if `.lwid.json` exists).
3. Parse the shareable URL from stdout.
4. Give the user the **edit URL** (includes write key) so they can push updates later.
5. If they want a view-only link for sharing, run `lwid info` and provide the view URL.

## Constraints

- Max project size: 10MB.
- All files in the directory are uploaded except hidden files, `node_modules`, and `.lwid.json`.
- The `.lwid.json` file is the only record of the keys — losing it means losing write access.
- The server never sees plaintext content.

## Persistent Storage

lwid apps can persist data (game scores, user preferences, uploaded files, etc.) using an encrypted key-value store. Both keys and values are encrypted client-side — the server never sees plaintext.

For full documentation on the store API, see [SKILL-store.md]({{SERVER_URL}}/SKILL-store.md).
