#!/usr/bin/env bash
# cli.sh: enable the app's API and gRPC servers so the node registers the
# tx service (Simulate, for --gas auto) on its query router.
set -euo pipefail
python3 - "$HOME_DIR/config/app.toml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
def section(s, name, f):
    i = s.index("[%s]" % name)
    j = s.find("\n[", i + 1)
    j = len(s) if j < 0 else j
    return s[:i] + f(s[i:j]) + s[j:]
for name in ("api", "grpc"):
    s = section(s, name, lambda b: b.replace("enable = false", "enable = true", 1))
open(p, "w").write(s)
PY
