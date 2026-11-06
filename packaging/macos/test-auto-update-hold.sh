#!/usr/bin/env bash
# The qualification hold must pause deployments without consulting Git.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/auto-update" "$t/bin"
cat >"$t/bin/git" <<'EOF'
#!/usr/bin/env bash
echo "git unexpectedly invoked" >&2
exit 42
EOF
chmod +x "$t/bin/git"
script=$here/celestia-rs-auto-update.sh

echo "$(( $(date +%s) + 300 ))" >"$t/auto-update/qualification-hold-until"
out=$(PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" "$script")
[[ $out == *"qualification hold: deployments paused"* ]]

echo invalid >"$t/auto-update/qualification-hold-until"
rc=0
PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" "$script" >"$t/out" 2>&1 || rc=$?
[[ $rc == 2 ]]
grep -q 'invalid qualification hold deadline' "$t/out"

# A digits-only value outside Bash's signed integer range must not wrap
# into an expired deadline and continue into Git.
echo 18446744073709551615 >"$t/auto-update/qualification-hold-until"
rc=0
PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" "$script" >"$t/out" 2>&1 || rc=$?
[[ $rc == 2 ]]
grep -q 'invalid qualification hold deadline' "$t/out"

echo "$(( $(date +%s) - 1 ))" >"$t/auto-update/qualification-hold-until"
rc=0
PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" "$script" >"$t/out" 2>&1 || rc=$?
[[ $rc == 42 ]]
grep -q 'git unexpectedly invoked' "$t/out"
echo "all auto-update hold tests passed"
