# Project names

*Design note — no implementation yet. The point of this document is the fork in
§2, which needs a decision before any code is worth writing.*

## 1. The ask

Projects are addressed by a nanoid: `/p/L2AB8l1Chrch`. That string is all a
person sees in the projects dropdown, and it tells them nothing about which of
their projects it is. They should be able to name a project, and a project
should arrive with a sensible default name rather than requiring one — the
`<title>` of its entry page, say.

## 2. The fork: a name you *see* vs. a name in the *URL*

"Instead of those weird IDs it would have a name" can mean two things, and they
are not variations on one feature. They differ in whether the server learns the
name.

**A. Display name.** The URL stays `/p/L2AB8l1Chrch#key`. The name is stored
encrypted, decrypted in the page, and shown everywhere a human looks: the
projects dropdown, the toolbar, the browser tab, the share dialog. The server
stores ciphertext and learns nothing.

**B. Vanity URL.** The URL becomes `/p/my-admin-console`. The server must read
that string to route the request, so the server — and every proxy, every access
log, every TLS SNI-adjacent observer of the request line — learns what the
project is called.

B is incompatible with the premise of this codebase. The server already goes to
some length to know as little as possible: file contents and *file paths* are
AES-256-GCM encrypted (`SCHEMA_ENCRYPTED_PATHS`), the decryption key never
leaves the fragment, and the manifest is plaintext only because it carries
nothing but sizes and CIDs. A plaintext human-meaningful name in the request
path would be the first piece of content the server has ever been told, and
"zero-knowledge" in the README would stop being true as written.

B also drags in problems A does not have: a global namespace means uniqueness,
collisions, squatting, reservation of system paths, and a rename story for
links already shared. None of that is hard; it is just a different, much larger
feature with a policy surface.

**Recommendation: build A.** It delivers what the complaint actually is —
a dropdown full of gibberish — and costs nothing in security. Keep B as a
separate decision, and if it is ever wanted, do it with eyes open as an
explicit opt-in per project whose dialog says plainly that the name will be
visible to the server.

The rest of this note is A.

## 3. Where the name lives

Three candidate homes. All three store ciphertext; they differ in *who can
change the name* and *what it travels with*.

### 3a. In the manifest — recommended

Add one optional field to `Manifest`, holding
`base64url(AES-256-GCM(read_key, name))` — the same wire format already used
for `FileEntry.path`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub name: Option<String>,
```

- **Integrity.** A manifest is published by signing its CID with the write key,
  so only someone who can publish a version can change the name. That matches
  how everything else in a project works.
- **It travels.** `lwid clone` and `lwid pull` already fetch the manifest, so
  the name arrives with the content, on every client, for free.
- **No schema bump.** `version >= 100` already means encrypted paths; an
  optional field with `serde(default)` is both backward and forward compatible.
  Old manifests simply have no name. Nothing needs `SCHEMA_ENCRYPTED_PATHS + 1`.
- **Cost:** renaming means publishing a new version, since the name is part of
  the snapshot. That is defensible — a rename *is* a change to the project —
  but it does put a no-content-change entry in the version history.

Pushes must carry the name forward or it would be lost on the next push.
`merge_with_existing()` already reads the parent manifest for selective pushes,
so the plumbing is there.

### 3b. In the encrypted KV store

There is already a precedent: the chosen entry point is stored under an
HMAC-obfuscated key with an encrypted value (`entry:{manifestCid}`). A `name`
key would work identically, needs no manifest change, and renaming would not
create a version.

The problem is who may write it. The store token is
`SHA-256("lwid-store-auth:" + read_key)` — **derived from the read key** — so
anyone holding a view-only link can write to the store. That is deliberate (it
is what makes the guestbook example work) but it means a read-only visitor
could rename someone else's project. For a label that appears in the owner's
own dropdown, that is a defacement vector with no upside.

### 3c. localStorage only

Rejected: it would not survive a different browser, would not reach anyone the
link is shared with, and would not be visible to the CLI. The dropdown is
already localStorage-backed, so this would look like it works right up until it
matters.

**Use 3a as the source of truth.** Cache the decrypted name in the existing
localStorage project record purely so the dropdown can render instantly — see
§5.

## 4. The default name

Derivation must happen where plaintext exists, which means the shell (it holds
decrypted files) and the CLI (it holds local files) — never the server. Both
already have the pieces: the shell has `resolveEntryPoint()`, the CLI walks the
directory before encrypting.

Order, first hit wins:

1. `<title>` of the resolved entry HTML, trimmed, collapsed whitespace.
2. First `<h1>` of that page.
3. Viewer-specific: the notebook's filename for `*.ipynb`; the `SUMMARY.md`
   title or the `README.md` first heading for a docs project.
4. On `lwid push`, the directory name.
5. Nothing — fall back to the truncated ID, exactly as today.

Capped at 80 characters (see §6). A default is a *suggestion written into the
manifest at creation*, not a value recomputed on every render — otherwise the
name would silently change when someone edits a heading.

## 5. Surfaces to change

| Surface | Today | After |
|---|---|---|
| Projects dropdown | `truncateId(p.id)` | name, ID as the subtitle / `title=` |
| Toolbar | `#toolbar-doc-title`, set only by viewers via `LWID_TITLE_SET` | same slot, defaulting to the project name for plain sites too |
| Browser tab | static | `document.title` = project name |
| `lwid info` | id, server, URLs | add name |
| `lwid push` | — | `--name`, and write the derived default on first push |
| `lwid clone` | clones into `<id>/` or a given dir | slugified name as the default directory |
| `SKILL.md` | — | tell agents to set a name; it is the one thing they are well placed to write |

The toolbar already has the mechanism: `LWID_TITLE_SET` → `setDocTitle()` →
`#toolbar-doc-title`. Today only viewers push to it and a plain HTML site
leaves it empty. The project name is the obvious default for that slot, with a
viewer's own title overriding it while that viewer is open.

Rename UI: an inline edit on the toolbar title, shown only when
`currentWriteKey` is set, since renaming publishes a version.

## 6. Security

**The name is attacker-controlled text rendered by the shell, and the shell has
no HTML escaping today.** There are 21 `innerHTML` assignments in
`shell/index.html` and no `escapeHtml` helper — the dropdown builds its markup
with a template string. Every value interpolated there so far has been a nanoid
or a number, so this has never mattered. A name is the first free-form string
to reach it.

This is the one part of the feature that can go wrong in a way that matters:
the shell's origin holds *other projects' write keys* in localStorage, so
script execution there is exactly the thing the per-project sandbox origin was
built to prevent. A project whose name is `<img src=x onerror=...>` must not be
able to reach that.

Therefore, non-negotiably:

- render names with `textContent`, or escape at a single chokepoint;
- cap length (80 chars) and strip control characters at the point the name is
  *set*, not where it is displayed;
- treat a name decrypted from a manifest as untrusted input even though it
  decrypted correctly — authenticity of the ciphertext says the write-key
  holder wrote it, not that it is safe to inject.

Worth adding the escaping helper regardless; the absence of one is a loaded gun
for the next person who interpolates a string.

## 7. Phases

1. **Manifest + CLI.** `name` on `Manifest` (Rust and JS), `--name` on push,
   default derivation, `lwid info`. No UI yet; verifiable with `lwid info` and
   a manifest dump.
2. **Shell display.** Dropdown, toolbar, tab title, localStorage cache for
   instant rendering. Includes the escaping chokepoint from §6.
3. **Rename UI.** Inline edit for write-key holders, publishing a version.
4. **Vanity URLs.** Separate decision; see §2.

Phases 1 and 2 are each independently shippable and each independently useful.

## 8. Open questions

- Is a rename worth a version in the history, or should the name move to the KV
  store and accept that a view-only link can change it? (§3a vs §3b)
- Should the default name be recomputed when the entry page's `<title>`
  changes, or stay fixed once written? This note assumes fixed.
- Does `lwid clone` renaming its target directory after the project name
  surprise anyone relying on the current `<id>/` behaviour?
