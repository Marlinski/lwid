#!/usr/bin/env bash
#
# push-with-openssl.sh — publish a project to lwid using only curl and
# openssl, with no lwid binary involved.
#
#   usage: scripts/push-with-openssl.sh [-s SERVER] [-t TTL] FILE...
#   prints the share URL, keys and all
#
# This exists as executable documentation of the client side of the
# protocol. Everything the real client does to a file before it leaves the
# machine is here, in the order it happens, in about a hundred lines. If the
# Rust client and this script ever disagree, one of them has a bug.
#
# Requires: bash 4+, curl, openssl 3.x (for `openssl mac`), coreutils.
#
# Deliberately *not* a replacement for `lwid push`: no chunking, no dedup,
# no incremental versions (every run starts a fresh project with
# parent_cid: null), and it shells out to openssl once per file.
#
# ---------------------------------------------------------------------------
# On AES-GCM without AES-GCM
# ---------------------------------------------------------------------------
# lwid encrypts with AES-256-GCM, and openssl's `enc` command refuses to do
# it at all:
#
#     $ openssl enc -aes-256-gcm ...
#     enc: AEAD ciphers not supported
#
# So GCM gets assembled from the pieces openssl does expose. The ciphertext
# is the easy half — GCM's encryption *is* CTR mode, starting at counter
# IV||00000002 (block 1 is reserved for the tag). The authentication tag is
# the interesting half, and the way in is GMAC: GMAC is GCM with all the
# data fed as additional authenticated data instead of as plaintext. Both
# compute
#
#     tag = ((GHASH over the data blocks) xor L) * H  xor  E(K, IV||00000001)
#
# over identical data blocks, differing only in L, the trailing block
# encoding the two input lengths:
#
#     GCM  (all plaintext): L = u64(0)        || u64(bitlen)
#     GMAC (all AAD):       L = u64(bitlen)   || u64(0)
#
# Since multiplication in GF(2^128) distributes over xor, the whole
# difference between the two tags is
#
#     (L_gcm xor L_gmac) * H  ==  (bitlen || bitlen) * H
#
# which is one multiply, regardless of how big the message is. openssl still
# does every AES block and every GHASH block; the shell does a single
# 128-bit carry-less multiply at the end to correct the length block.
#
set -euo pipefail

SERVER=${LWID_SERVER:-https://lookwhatidid.xyz}
TTL=7d

usage() {
  cat <<EOF
usage: $0 [-s SERVER] [-t TTL] FILE...

Encrypt FILEs and publish them as an lwid project, using only curl and
openssl. Prints the share URL, with the read and write keys in the fragment.

  -s SERVER   server base URL (default \$LWID_SERVER, else $SERVER)
  -t TTL      1h | 1d | 7d | 30d | never  (default $TTL)
EOF
}

while getopts ':s:t:h' opt; do
  case $opt in
    s) SERVER=$OPTARG ;;
    t) TTL=$OPTARG ;;
    h) usage; exit 0 ;;
    *) echo "unknown option -$OPTARG (try -h)" >&2; exit 2 ;;
  esac
done
shift $((OPTIND - 1))
[ $# -gt 0 ] || { usage >&2; exit 2; }

for tool in curl openssl base32 od; do
  command -v "$tool" >/dev/null || { echo "missing required tool: $tool" >&2; exit 1; }
done
openssl list -mac-algorithms 2>/dev/null | grep -q GMAC \
  || { echo "this openssl has no GMAC; need openssl 3.x" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Encoding helpers
# ---------------------------------------------------------------------------
b64std()  { openssl base64 -A; }                              # -> standard base64
b64url()  { openssl base64 -A | tr '+/' '-_' | tr -d '='; }   # -> base64url, unpadded
bin2hex() { od -An -v -tx1 | tr -d ' \n'; }
# Only ever used on a 12-byte IV and a 16-byte tag, so passing hex as an
# argument is safe; file contents stay binary from end to end.
hex2bin() { printf '%b' "$(sed 's/../\\x&/g' <<<"$1")"; }

# CID: multibase-base32lower("b") of CIDv1 || raw codec || sha2-256 multihash.
# All four varints are single-byte, so the binary form is a fixed 4-byte
# prefix followed by the digest — 36 bytes, which base32 renders in 58
# characters with no padding.
cid_of() {
  { printf '\x01\x55\x12\x20'; openssl dgst -sha256 -binary "$1"; } \
    | base32 -w0 | tr -d '=' | tr 'A-Z' 'a-z' | sed 's/^/b/'
}

# ---------------------------------------------------------------------------
# Multiplication in GF(2^128), GCM's bit convention (bit 0 is the MSB).
# Operands and result are 32-char hex strings, held as two 64-bit halves
# because that is the widest integer the shell has.
# ---------------------------------------------------------------------------
gf_mul() {
  local xh=$((0x${1:0:16})) xl=$((0x${1:16:16}))
  local vh=$((0x${2:0:16})) vl=$((0x${2:16:16}))
  local zh=0 zl=0 i bit lsb
  for ((i = 0; i < 128; i++)); do
    if ((i < 64)); then bit=$(((xh >> (63 - i)) & 1)); else bit=$(((xl >> (127 - i)) & 1)); fi
    if ((bit)); then ((zh ^= vh, zl ^= vl)); fi
    lsb=$((vl & 1))
    # v >>= 1 across the full 128 bits. The mask turns the shell's
    # arithmetic shift into the logical one this needs.
    vl=$((((vl >> 1) & 0x7FFFFFFFFFFFFFFF) | ((vh & 1) << 63)))
    vh=$(((vh >> 1) & 0x7FFFFFFFFFFFFFFF))
    # Reduce modulo the GCM polynomial when a 1 was shifted out.
    if ((lsb)); then ((vh ^= 0xe100000000000000)); fi
  done
  printf '%016x%016x' "$zh" "$zl"
}

# ---------------------------------------------------------------------------
# AES-256-GCM. Writes iv || ciphertext || tag — exactly the blob layout lwid
# stores, and what the browser hands to crypto.subtle.decrypt.
# ---------------------------------------------------------------------------
gcm_encrypt() {
  local key_hex=$1 infile=$2 outfile=$3
  local iv h tag_gmac bitlen lenblock fix tag ct="$TMP/.ct"

  iv=$(openssl rand -hex 12)
  # GCM's counter starts one past J0 = iv||00000001.
  openssl enc -aes-256-ctr -K "$key_hex" -iv "${iv}00000002" -in "$infile" -out "$ct"
  # H, the GHASH subkey: the cipher applied to a zero block.
  h=$(head -c 16 /dev/zero | openssl enc -aes-256-ecb -K "$key_hex" -nopad | bin2hex)
  # GMAC does all the per-block GHASH work over the ciphertext.
  tag_gmac=$(openssl mac -macopt "hexkey:$key_hex" -macopt "hexiv:$iv" \
    -cipher aes-256-gcm -in "$ct" GMAC | tr 'A-Z' 'a-z')

  # Correct GMAC's length block into GCM's (see the header comment). CTR is a
  # stream mode, so the ciphertext is exactly as long as the plaintext.
  bitlen=$(( $(wc -c <"$ct") * 8 ))
  lenblock=$(printf '%016x%016x' "$bitlen" "$bitlen")
  fix=$(gf_mul "$lenblock" "$h")
  tag=$(printf '%016x%016x' \
    $(( 0x${tag_gmac:0:16} ^ 0x${fix:0:16} )) \
    $(( 0x${tag_gmac:16:16} ^ 0x${fix:16:16} )) )

  { hex2bin "$iv"; cat "$ct"; hex2bin "$tag"; } > "$outfile"
  rm -f "$ct"
}

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# Keys. Neither of these ever reaches the server: they travel in the URL
# fragment, which browsers do not send.
# ---------------------------------------------------------------------------
# Read key — the AES-256 key, shared with anyone given the link.
openssl rand 32 > "$TMP/read.key"
READ_KEY_HEX=$(bin2hex < "$TMP/read.key")
READ_KEY=$(b64url < "$TMP/read.key")

# Write key — Ed25519. lwid keeps only the 32-byte seed; a PKCS#8 private key
# is a fixed 16-byte header followed by exactly that seed, and an SPKI public
# key a fixed 12-byte header followed by the 32-byte point, so the raw
# material is just the tail of each DER encoding.
openssl genpkey -algorithm ED25519 -out "$TMP/write.pem" 2>/dev/null
WRITE_KEY=$(openssl pkey -in "$TMP/write.pem" -outform DER | tail -c 32 | b64url)
WRITE_PUBKEY=$(openssl pkey -in "$TMP/write.pem" -pubout -outform DER | tail -c 32 | b64std)

# Store token — authenticates the per-project key/value store. Derived from
# the read key so that holding the link is what grants access.
STORE_TOKEN=$(printf 'lwid-store-auth:%s' "$READ_KEY" | openssl dgst -sha256 -binary | b64std)

json_field() { sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }

# ---------------------------------------------------------------------------
# 1. Create the project. The server learns the write public key (so it can
#    verify later updates) and nothing else of consequence.
# ---------------------------------------------------------------------------
PROJECT_ID=$(curl -fsS -X POST "$SERVER/api/projects" \
  -H 'Content-Type: application/json' \
  -d "$(printf '{"write_pubkey":"%s","ttl":"%s","store_token":"%s","client_version":"push-with-openssl"}' \
        "$WRITE_PUBKEY" "$TTL" "$STORE_TOKEN")" | json_field project_id)
[ -n "$PROJECT_ID" ] || { echo "could not create project on $SERVER" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 2. Encrypt and upload each file. Both the contents and the path are
#    ciphertext; only the size stays visible.
# ---------------------------------------------------------------------------
ENTRIES=""
for f in "$@"; do
  [ -f "$f" ] || { echo "not a file: $f" >&2; exit 1; }
  rel=${f#./}

  gcm_encrypt "$READ_KEY_HEX" "$f" "$TMP/blob.bin"
  blob_cid=$(curl -fsS -X POST "$SERVER/api/blobs" \
    -H 'Content-Type: application/octet-stream' \
    --data-binary "@$TMP/blob.bin" | json_field cid)

  # The CID is a pure function of the bytes, so both sides must derive the
  # same one. Checking it here turns a silent broken manifest into an error.
  local_cid=$(cid_of "$TMP/blob.bin")
  [ "$blob_cid" = "$local_cid" ] \
    || { echo "CID mismatch for $rel: server $blob_cid, local $local_cid" >&2; exit 1; }

  printf '%s' "$rel" > "$TMP/path.txt"
  gcm_encrypt "$READ_KEY_HEX" "$TMP/path.txt" "$TMP/path.enc"
  enc_path=$(b64url < "$TMP/path.enc")

  ENTRIES="${ENTRIES:+$ENTRIES,}$(printf '{"path":"%s","cid":"%s","size":%s}' \
    "$enc_path" "$blob_cid" "$(wc -c <"$f" | tr -d ' ')")"
  echo "  encrypted $rel -> $blob_cid" >&2
done

# ---------------------------------------------------------------------------
# 3. The manifest ties the version together. It is stored as plaintext JSON
#    on purpose: the server needs the sizes and blob CIDs to enforce quotas
#    and garbage-collect, and those leak nothing once paths and contents are
#    encrypted. Schema 100 is what marks the paths as encrypted.
# ---------------------------------------------------------------------------
printf '{"version":100,"parent_cid":null,"timestamp":"%s","files":[%s]}' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$ENTRIES" > "$TMP/manifest.json"
ROOT_CID=$(curl -fsS -X POST "$SERVER/api/blobs" \
  -H 'Content-Type: application/octet-stream' \
  --data-binary "@$TMP/manifest.json" | json_field cid)

# ---------------------------------------------------------------------------
# 4. Publish it, signing the root CID with the write key. The signature is
#    over the CID's ASCII bytes and nothing else — no nonce, no timestamp —
#    so there is no canonical form to get wrong.
# ---------------------------------------------------------------------------
printf '%s' "$ROOT_CID" > "$TMP/root.cid"
# -rawin insists on a seekable input: Ed25519 signs in one shot, so openssl
# wants the length up front and will not read a pipe.
SIG=$(openssl pkeyutl -sign -inkey "$TMP/write.pem" -rawin -in "$TMP/root.cid" | b64std)
curl -fsS -o /dev/null -X PUT "$SERVER/api/projects/$PROJECT_ID/root" \
  -H 'Content-Type: application/json' \
  -d "$(printf '{"root_cid":"%s","signature":"%s"}' "$ROOT_CID" "$SIG")"

printf '%s/p/%s#%s:%s\n' "$SERVER" "$PROJECT_ID" "$READ_KEY" "$WRITE_KEY"
