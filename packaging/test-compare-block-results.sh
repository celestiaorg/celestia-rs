#!/usr/bin/env bash
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin"
cat >"$t/bin/curl" <<'EOF'
#!/usr/bin/env bash
url=${!#}
# Exercise malformed responses on either endpoint, including identical
# malformed payloads that must never count as a successful comparison.
if [[ $url == *'/block_results?'* && -n ${TEST_RESPONSE:-} &&
      ( ${TEST_SIDE:-both} == both || $url == "http://${TEST_SIDE}/"* ) ]]; then
  printf '%s\n' "$TEST_RESPONSE"
  exit 0
fi
case $url in
  http://local/status) echo '{"result":{"node_info":{"network":"celestia"}}}' ;;
  http://ref/status)
    if [[ ${TEST_CASE:-} == wrong_chain ]]; then chain=other; else chain=celestia; fi
    printf '{"result":{"node_info":{"network":"%s"}}}\n' "$chain" ;;
  'http://local/block_results?height=42'|'http://local/block_results?height=43')
    height=${url##*=}
    printf '{"result":{"height":"%s","txs_results":[{"code":0,"data":"AQ==","gas_wanted":"12","gas_used":"10"}]}}\n' "$height" ;;
  'http://ref/block_results?height=42'|'http://ref/block_results?height=43')
    height=${url##*=}
    [[ ${TEST_CASE:-} == unavailable || ( ${TEST_CASE:-} == mixed && $height == 42 ) ]] && exit 22
    if [[ ( ${TEST_CASE:-} == mismatch || ${TEST_CASE:-} == mixed ) && $height == 43 ]]; then gas=11; else gas=10; fi
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

rc=0
PATH="$t/bin:$PATH" TEST_CASE=mixed "$check" http://local http://ref 42 43 "$t/mixed.jsonl" >/dev/null || rc=$?
[[ $rc == 2 ]]
jq -es '.[0].outcome=="unavailable" and .[1].outcome=="mismatch"' "$t/mixed.jsonl" >/dev/null

for side in local ref both; do
  for response in \
    '{"result":{"height":"42","txs_results":[null]}}' \
    '{"result":{"height":"42","txs_results":[{"code":false}]}}' \
    '{"result":{"height":"42","txs_results":[{"code":0.5}]}}' \
    '{"result":{"height":"42","txs_results":[{"code":-1}]}}' \
    '{"result":{"height":"42","txs_results":[{"code":4294967296}]}}' \
    '{"result":{"height":"42","txs_results":[{"gas_used":"unknown"}]}}' \
    '{"result":{"height":"42","txs_results":[{"gas_wanted":{}}]}}' \
    '{"result":{"height":"42","txs_results":[{"data":true}]}}' \
    '{"result":{"height":"42","txs_results":[{"data":"invalid base64"}]}}' \
    '{"error":{"code":-32603},"result":{"height":"42","txs_results":[]}}'; do
    rm -f "$t/invalid.jsonl"
    rc=0
    PATH="$t/bin:$PATH" TEST_SIDE="$side" TEST_RESPONSE="$response" \
      "$check" http://local http://ref 42 42 "$t/invalid.jsonl" >/dev/null || rc=$?
    [[ $rc == 2 ]]
    if [[ $side == ref ]]; then endpoint=reference; else endpoint=local; fi
    jq -es --arg error "invalid_${endpoint}_block_results" \
      'length==1 and .[0].outcome=="unavailable" and .[0].error==$error and
       .[0].local_results_sha256==null and .[0].reference_results_sha256==null' \
      "$t/invalid.jsonl" >/dev/null
  done
done

# Empty blocks and omitted/default fields are valid Go JSON responses.
for results in null '[]' '[{}]' '[{"code":null,"data":null,"gas_wanted":null,"gas_used":null}]'; do
  rm -f "$t/defaults.jsonl"
  PATH="$t/bin:$PATH" TEST_RESPONSE="{\"result\":{\"height\":\"42\",\"txs_results\":$results}}" \
    "$check" http://local http://ref 42 42 "$t/defaults.jsonl" >/dev/null
  jq -e '.outcome=="match"' "$t/defaults.jsonl" >/dev/null
done
echo "all block-result receipt tests passed"
