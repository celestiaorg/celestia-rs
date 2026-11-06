#!/usr/bin/env bash
# Check receipts, mismatch classification, and unavailable references.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin"
cat >"$t/bin/curl" <<'EOF'
#!/usr/bin/env bash
url=${!#}
case $url in
  http://local/abci_info)
    printf '{"result":{"response":{"last_block_height":"42","last_block_app_hash":"%s"}}}\n' \
      'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' ;;
  http://local/status) echo '{"result":{"node_info":{"network":"celestia"}}}' ;;
  http://ref/status)
    if [[ ${TEST_CASE:-} == unavailable ]]; then exit 22; fi
    if [[ ${TEST_CASE:-} == wrong_chain ]]; then chain=other; else chain=celestia; fi
    printf '{"result":{"node_info":{"network":"%s"}}}\n' "$chain" ;;
  'http://ref/block?height=43')
    if [[ ${TEST_CASE:-} == mismatch ]]; then hash=$(printf 'F%.0s' {1..64})
    else hash=$(printf '0%.0s' {1..64}); fi
    printf '{"result":{"block":{"header":{"chain_id":"celestia","app_hash":"%s"}}}}\n' "$hash" ;;
  *) exit 22 ;;
esac
EOF
chmod +x "$t/bin/curl"
check=$here/check-apphash.sh

PATH="$t/bin:$PATH" "$check" http://local http://ref >"$t/text"
grep -q '^OK height=42 app_hash=' "$t/text"
PATH="$t/bin:$PATH" CANDIDATE_COMMIT=abc123 "$check" http://local http://ref --json >"$t/match"
jq -e '.outcome=="match" and .chain_id=="celestia" and
  .reference_chain_id=="celestia" and .executed_height==42 and
  .reference_header_height==43 and .candidate_commit=="abc123" and
  .local_app_hash==.reference_app_hash' "$t/match" >/dev/null

rc=0
PATH="$t/bin:$PATH" TEST_CASE=mismatch "$check" http://local http://ref --json >"$t/mismatch" || rc=$?
[[ $rc == 1 ]] || { echo "mismatch exit status: $rc" >&2; exit 1; }
jq -e '.outcome=="mismatch" and .local_app_hash!=.reference_app_hash' "$t/mismatch" >/dev/null

for case in unavailable wrong_chain; do
  rc=0
  PATH="$t/bin:$PATH" TEST_CASE="$case" "$check" http://local http://ref --json >"$t/$case" || rc=$?
  [[ $rc == 2 ]] || { echo "$case exit status: $rc" >&2; exit 1; }
  jq -e '.outcome=="unavailable" and .error!=null' "$t/$case" >/dev/null
done
echo "all app-hash receipt tests passed"
