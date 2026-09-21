#!/bin/sh
# Optionally inject an analytics snippet into lwid's own pages, then run the
# server.
#
# With UMAMI_WEBSITE_ID unset nothing is touched and the shell is served exactly
# as it was built. That is the off switch, and also what happens if a deployment
# never sets the variable.
#
# WHAT IS DELIBERATELY NOT TOUCHED
#
#   __lwid_sandbox__/ and viewers/   These render user-uploaded, end-to-end
#                                    encrypted content, on their own origin
#                                    (<id>.p.lookwhatidid.xyz). lwid's whole
#                                    claim is that the server never sees your
#                                    bytes; reporting on who opens which private
#                                    payload would break that, so the tracker
#                                    stays out of the sandbox entirely.
#
#   The second </head> in index.html Ships inside the STARTER_WEBSITE template
#                                    literal — the starter page handed to every
#                                    new project. Injecting there would embed
#                                    this tracker in apps that users publish.
#                                    Hence first-match-only substitution.
set -eu

if [ -n "${UMAMI_WEBSITE_ID:-}" ]; then
  SHELL_DIR="${LWID_SERVER__SHELL_DIR:-/shell}"
  URL="${UMAMI_SCRIPT_URL:-https://stats.marlinski.org/script.js}"
  SNIPPET="<script defer src=\"${URL}\" data-website-id=\"${UMAMI_WEBSITE_ID}\"></script>"

  for page in index.html docs.html terms.html; do
    f="${SHELL_DIR}/${page}"
    [ -f "$f" ] || continue
    grep -q 'data-website-id' "$f" && continue   # already done, container restart
    # 0,/pattern/ bounds the substitution to the FIRST match only.
    sed -i "0,\|</head>|s||${SNIPPET}</head>|" "$f"
    echo "umami: injected into ${page}"
  done
  echo "umami: website ${UMAMI_WEBSITE_ID} (${URL}); sandbox and viewers untouched"
else
  echo "umami: UMAMI_WEBSITE_ID unset — analytics disabled, shell served unmodified"
fi

exec lwid-server "$@"
