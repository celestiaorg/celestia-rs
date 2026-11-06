#!/usr/bin/env bash
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin"
cat >"$t/bin/curl" <<'EOF'
#!/usr/bin/env bash
url=${!#}
case $url in
  http://local/status) echo '{"result":{"node_info":{"network":"celestia"}}}' ;;
  http://ref/status)
    if [[ ${TEST_CASE:-} == wrong_chain ]]; then chain=other; else chain=celestia; fi
    printf '{"result":{"node_info":{"network":"%s"}}}\n' "$chain" ;;
  'http://local/block_results?height=42'|'http://local/block_results?height=43')
    height=${url##*=}
    printf '{"result":{"height":"%s","txs_results":[{"code":0,"data":"AQ==","gas_wanted":"12","gas_used":"10"}]}}\n' "$height" ;;
  'http://ref/block_results?height=42'|'http://ref/block_results?height=43')
    [[ ${TEST_CASE:-} == unavailable ]] && exit 22
    height=${url##*=}
    if [[ ${TEST_CASE:-} == mismatch && $height == 43 ]]; then gas=11; else gas=10; fi
    printf '{"result":{"height":"%s","txs_results":[{"gas_used":"%s","gas_wanted":"12","data":"AQ=="}]}}\n' "$height" "$gas" ;;
  *) exit 22 ;;
esac
EOF
chmod +x "$t/bin/curl"
check=$here/compare-block-results.sh

PATH="$t/bin:$PATH" "$check" http://local http://ref 42 43 "$t/match.jsonl" abc123 >/dev/null
[[ $(wc -l <"$t/match.jsonl" | tr -d ' ') == 2 ]]
jq -es 'length==2 and all(.[]; .outcome=="match" and
  .candidate_commit=="abc123" and .local_tx_count==1 and
  .local_results_sha256==.reference_results_sha256)' "$t/match.jsonl" >/dev/null
if PATH="$t/bin:$PATH" "$check" http://local http://ref 42 43 "$t/match.jsonl" >/dev/null 2>&1; then
  echo "existing receipt file was overwritten" >&2
  exit 1
fi
[[ $(wc -l <"$t/match.jsonl" | tr -d ' ') == 2 ]]

rc=0
PATH="$t/bin:$PATH" TEST_CASE=mismatch "$check" http://local http://ref 42 43 "$t/mismatch.jsonl" >/dev/null || rc=$?
[[ $rc == 1 ]]
jq -es '.[0].outcome=="match" and .[1].outcome=="mismatch" and
  .[1].first_difference_index==0 and
  .[1].local_results_sha256!=.[1].reference_results_sha256' "$t/mismatch.jsonl" >/dev/null

for case in unavailable wrong_chain; do
  rc=0
  PATH="$t/bin:$PATH" TEST_CASE="$case" "$check" http://local http://ref 42 43 "$t/$case.jsonl" >/dev/null || rc=$?
  [[ $rc == 2 ]]
  jq -es 'length==2 and all(.[]; .outcome=="unavailable" and .error!=null)' "$t/$case.jsonl" >/dev/null
done
echo "all block-result receipt tests passed"
