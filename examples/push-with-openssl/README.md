# push-with-openssl (example project)

A one-page write-up of how [`scripts/push-with-openssl.sh`](../../scripts/push-with-openssl.sh)
reconstructs AES-256-GCM out of the pieces `openssl` is willing to perform —
with the script's full source embedded in the page.

`index.html` is generated; edit `template.html` (the prose and styling) or the
script itself, then regenerate:

```sh
examples/push-with-openssl/build.sh
```

The listing is injected at build time rather than pasted so it cannot drift
from the script it documents, and so the HTML escaping is never done by hand.

Publish it with the script it describes:

```sh
scripts/push-with-openssl.sh -s https://lookwhatidid.xyz examples/push-with-openssl/index.html
```

Because the project contains an `index.html`, lwid serves it as an ordinary
static site rather than opening it through a viewer.
