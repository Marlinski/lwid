#!/usr/bin/env sh
# Regenerate index.html by embedding the real push script into template.html.
#
# The embedded listing is generated rather than pasted so it cannot drift
# from the script it documents, and so the HTML escaping is never done by
# hand — the script is full of <, > and & and a single missed entity would
# corrupt the page silently.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
script=$here/../../scripts/push-with-openssl.sh

[ -f "$script" ] || { echo "missing $script" >&2; exit 1; }

python3 - "$script" "$here/template.html" "$here/index.html" <<'PY'
import html, re, sys

script_path, template_path, out_path = sys.argv[1:4]
src = open(script_path, encoding="utf-8").read()

# Escape first, then add spans: highlighting must not escape its own markup.
esc = html.escape(src, quote=False)
esc = re.sub(r"(?m)(^|\s)(#(?!\{).*)$", lambda m: f"{m.group(1)}<span class=\"c\">{m.group(2)}</span>", esc)
esc = re.sub(
    r"\b(if|then|else|elif|fi|for|do|done|while|case|esac|function|local|return|exit"
    r"|set|shift|printf|echo|command|openssl|curl|base32|od|tr|sed|cat|wc|date|mktemp|head|rm)\b",
    r'<span class="k">\1</span>',
    esc,
)

page = open(template_path, encoding="utf-8").read()
if "__SCRIPT__" not in page:
    sys.exit("template.html has no __SCRIPT__ placeholder")
open(out_path, "w", encoding="utf-8").write(page.replace("__SCRIPT__", esc))
print(f"wrote {out_path} ({len(esc.splitlines())} lines of script embedded)")
PY
