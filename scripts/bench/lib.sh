# shellcheck shell=bash disable=SC2034 # constants are used by vm.sh and the workflow
# Pure helpers for the operator sync comparison (docs/benchmarks/operator-sync.md).
# Sourced by vm.sh (on the benchmark VM), by the operator-benchmark workflow
# and by test.sh. No function here touches the network or the cloud.

# Fixed comparison inputs. Changing any of them starts a new series.
BENCH_GO_TAG=v11.0.0
BENCH_GO_COMMIT=58be8ae90d00518800df5506b3a1e988f009c1c6
BENCH_NODE_TAG=v0.36.0 # celestia-node built against celestia-app v11.0.0
BENCH_NODE_COMMIT=d5dfb21a0885622828e30e8e2daf50e5d1bb72b2
BENCH_GOLANG=go1.26.10 # go.mod of both Go repositories
BENCH_GOLANG_SHA256=ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5
BENCH_GENESIS_URL=https://raw.githubusercontent.com/celestiaorg/networks/master/celestia/genesis.json
BENCH_GENESIS_SHA256=9727aac9bbfb021ce7fc695a92f901986421283a891b89e0af97bc9fad187793
BENCH_REF_RPCS=(https://rpc.celestia.pops.one https://celestia-rpc.polkachu.com)
# Seeds and peers built into celestia-rs (crates/tier-5-node/celestia-rs/src/networks.rs).
BENCH_SEEDS=acca7837e4eb5f9dc7f5a94ed1d82edda6931ff8@seed.celestia.pops.one:26656,12ad7c73c7e1f2460941326937a039139aa78884@celestia-mainnet-seed.itrocket.net:40656,975237ec35350f0643bce6618654cd1c76bf1dc0@consensus-full-seed-1.celestia-bootstrap.net:26656
BENCH_PEERS=d9bfa29e0cf9c4ce0cc9c26d98e5d97228f93b0b@65.109.88.38:12056,fc25a389790d9f1e8dc39496313e1e0fe2b138f7@135.181.138.95:2890,d513f98bb4f8859f94bde2efc6bf18d29f732c48@188.40.102.137:15656,ce99d7da2530f75d05880d13d9eda384d5a1afe4@peer.celestia.node75.org:23356
BENCH_BRIDGE_HEIGHTS=600
BENCH_BRIDGE_STRIDE=20

# bench_claim_driver STATE: cross-process test-and-set for a VM startup script.
# The guest agent can invoke the same metadata script concurrently at boot.
bench_claim_driver() { mkdir "$1/started" 2>/dev/null; }

# bench_valid KIND VALUE: strict format check for every external input.
bench_valid() {
  local peer='[0-9a-f]{40}@[A-Za-z0-9.-]{1,253}:[0-9]{1,5}'
  case $1 in
    campaign) [[ $2 =~ ^(genesis-prefix|state-sync|bridge|block-range)$ ]] ;;
    blocks) [[ $2 =~ ^[1-9][0-9]{0,3}$ ]] ;;
    disk) [[ $2 =~ ^(pd|nvme)$ ]] ;;
    role) [[ $2 =~ ^(rs|go)$ ]] ;;
    height | trust_height) [[ $2 =~ ^[1-9][0-9]{0,7}$ ]] ;;
    pairs) [[ $2 =~ ^[1-3]$ ]] ;;
    minutes) [[ $2 =~ ^[1-9][0-9]{0,2}$ ]] ;;
    commit) [[ $2 =~ ^[0-9a-f]{40}$ ]] ;;
    run_id | deadline) [[ $2 =~ ^[0-9]{1,20}$ ]] ;;
    trust_hash) [[ $2 =~ ^[0-9A-F]{64}$ ]] ;;
    peers) [[ -z $2 || ($2 =~ ^$peer(,$peer){0,31}$) ]] ;;
    *) return 1 ;;
  esac
}

# bench_tip < polls: lines "elapsed local_height ref_height catching_up ...".
# A node is at tip when catching_up is false and it is within one block of
# the reference on two consecutive polls with advancing local heights.
# Prints "elapsed height" of the first such poll; fails if none.
bench_tip() {
  awk '
    function ok(l, r, c) { return l ~ /^[0-9]+$/ && r ~ /^[0-9]+$/ && c == "false" && (l - r <= 1 && r - l <= 1) }
    { q = ok($2, $3, $4)
      if (q && prev && $2 > pl) { print pt, pl; found = 1; exit }
      prev = q; pt = $1; pl = $2 }
    END { exit !found }'
}

# Compare the recorded snapshot heights after verifying both runs.
bench_pair_check() {
  if [[ ! $1 =~ ^[1-9][0-9]*$ || ! $2 =~ ^[1-9][0-9]*$ ]]; then echo discard:missing_snapshot; return 1; fi
  if [[ $1 != "$2" ]]; then echo discard:different_snapshots; return 1; fi
  echo valid
}

# bench_pair_verdict CAMPAIGN RUST_SUMMARY GO_SUMMARY: print one verdict even
# during an early collect, when one VM may not have published its summary yet.
bench_pair_verdict() {
  local campaign=$1 summary role=rs verdict
  local -a snapshots=()
  case $campaign in
    genesis-prefix) echo not_applicable; return 0 ;;
    state-sync | bridge | block-range) ;;
    *) echo discard:unknown_campaign; return 1 ;;
  esac
  for summary in "$2" "$3"; do
    if ! jq -se --arg role "$role" --arg campaign "$campaign" '
      length == 1 and (.[0] | .role == $role and .campaign == $campaign)
    ' "$summary" >/dev/null 2>&1; then
      echo discard:missing_or_invalid_summary; return 1
    fi
    if ! jq -e '.outcome == "ok"' "$summary" >/dev/null; then
      echo discard:incomplete_run; return 1
    fi
    if ! jq -e 'any(.verify[]?;
      .match == true and .chain_id == "celestia" and
      (.height | tostring | test("^[1-9][0-9]*$")) and
      (.local_block_id | type == "string" and test("^[0-9a-fA-F]{64}$")) and
      .local_block_id == .reference_block_id)' "$summary" >/dev/null 2>&1; then
      echo discard:unverified_tip; return 1
    fi
    snapshots+=("$(jq -r '.snapshot // empty' "$summary")")
    role=go
  done
  verdict=$(bench_pair_check "${snapshots[@]}") || { echo "$verdict"; return 1; }
  if [[ $campaign == bridge ]]; then
    verdict=$(jq -nr --slurpfile rs "$2" --slurpfile go "$3" \
      --argjson heights "$BENCH_BRIDGE_HEIGHTS" --argjson stride "$BENCH_BRIDGE_STRIDE" '
      def positive_height: type == "number" and . > 0 and . == floor;
      def complete:
        (.tail | positive_height) and (.start | positive_height) and (.end | positive_height) and
        .tail <= .start and .end == .start + $heights and
        .requests == (2 + $heights / $stride) and
        (.digest | type == "string" and test("^[0-9a-f]{64}$"));
      $rs[0].bridge as $r | $go[0].bridge as $g |
      if ([$r, $g] | all(.[]; complete) | not) then "discard:incomplete_bridge_receipt"
      elif [$r.tail, $r.start, $r.end] != [$g.tail, $g.start, $g.end] then "discard:different_da_ranges"
      elif $r != $g then "discard:different_workloads"
      else "valid" end') || { echo discard:incomplete_bridge_receipt; return 1; }
  fi
  if [[ $campaign == block-range ]]; then
    # Both nodes must cover the same scheduled window and stop at the same
    # committed height. A height gap is reported, never treated as equal.
    verdict=$(jq -nr --slurpfile rs "$2" --slurpfile go "$3" '
      def positive_height: type == "number" and . > 0 and . == floor;
      def complete:
        type == "object" and (.scheduled_start | positive_height) and (.blocks | positive_height) and
        (.entry_h | positive_height) and (.exit_h | positive_height) and (.final_app_height | positive_height) and
        .entry_h >= .scheduled_start and .exit_h >= .scheduled_start + .blocks;
      $rs[0].range as $r | $go[0].range as $g |
      if ([$r, $g] | all(.[]; complete) | not) then "discard:incomplete_range"
      elif [$r.scheduled_start, $r.blocks] != [$g.scheduled_start, $g.blocks] then "discard:different_range"
      elif $r.final_app_height != $g.final_app_height then
        "discard:different_final_heights:rs=\($r.final_app_height),go=\($g.final_app_height),diff=\($r.final_app_height - $g.final_app_height)"
      else "valid" end') || { echo discard:incomplete_range; return 1; }
  fi
  echo "$verdict"
  [[ $verdict == valid ]]
}

# bench_chunk FILE PREFIX [SIZE] [MAX]: gzip and base64 FILE into PREFIX.0,
# PREFIX.1, ... of at most SIZE bytes each (guest attribute values are
# limited to 256 KiB). Prints the count; fails above MAX chunks.
bench_chunk() {
  local size=${3:-196608} max=${4:-8} n
  gzip -9c "$1" | base64 -w0 | split -d -a 3 -b "$size" - "$2.tmp."
  n=0
  for f in "$2".tmp.*; do mv "$f" "$2.$n"; n=$((n + 1)); done
  if ((n > max)); then rm -f "$2".*; echo "too many chunks: $n > $max" >&2; return 1; fi
  echo "$n"
}

# bench_unchunk COUNT PREFIX: inverse of bench_chunk, to stdout.
bench_unchunk() {
  local i
  for ((i = 0; i < $1; i++)); do cat "$2.$i"; done | base64 -d | gunzip
}

# bench_log_excerpt: stdin is a node log. Prints its first 200 lines, then
# later lines about state sync, snapshots, the light client, block sync or
# problems: the first 1500 and the last 1500 of them, with a marker between.
# A whole node log is too large to publish.
bench_log_excerpt() {
  awk 'NR <= 200 { print; next }
       tolower($0) ~ /snapshot|statesync|state sync|light|blocksync|block sync|chunk|seed|error|warn|panic|fatal/ {
         if (n < 1500) print; else tail[n % 1500] = $0
         n++
       }
       END {
         if (n > 3000) printf "... %d matching lines omitted ...\n", n - 3000
         for (i = (n > 3000 ? n - 1500 : 1500); i < n; i++) print tail[i % 1500]
       }'
}

# bench_toml_set FILE SECTION KEY VALUE: set KEY (VALUE is a TOML literal)
# in [SECTION] ("" = top level), appending the key or section if absent.
bench_toml_set() {
  local tmp
  tmp=$(mktemp)
  awk -v sec="$2" -v key="$3" -v val="$4" '
    BEGIN { in_sec = (sec == "") }
    function put() { if (in_sec && !done) { print key " = " val; done = 1 } }
    /^\[/ { put(); printf "%s", blank; blank = ""; in_sec = ($0 == "[" sec "]"); print; next }
    /^[[:space:]]*$/ { blank = blank $0 "\n"; next }
    { printf "%s", blank; blank = "" }
    in_sec && !done && $1 == key && $2 == "=" { print key " = " val; done = 1; next }
    { print }
    END { put(); printf "%s", blank; if (!done) printf "\n[%s]\n%s = %s\n", sec, key, val }
  ' "$1" >"$tmp" && cat "$tmp" >"$1"
  rm -f "$tmp"
}

# bench_peers NET_INFO_JSON [LIMIT]: persistent peers from a /net_info
# response, IPv4 and well-formed IDs only.
bench_peers() {
  jq -r --argjson n "${2:-20}" '[.result.peers[]
      | {id: .node_info.id, ip: .remote_ip, port: (.node_info.listen_addr | split(":") | last)}
      | select((.id | test("^[0-9a-f]{40}$")) and (.ip | test("^[0-9]{1,3}(\\.[0-9]{1,3}){3}$")) and (.port | test("^[0-9]{1,5}$")))
      | "\(.id)@\(.ip):\(.port)"][:$n] | join(",")' "$1"
}

# bench_max FILE COL: numeric maximum of a whitespace-separated column.
bench_max() {
  awk -v c="$2" '$c ~ /^[0-9]+(\.[0-9]+)?$/ && (!s || $c + 0 > m) { m = $c + 0; s = 1 } END { if (s) printf "%.0f\n", m }' "$1"
}

# bench_range_stats FILE: JSON statistics of a range-memory.tsv (columns
# uptime_s local_h mem_current_B rss_KiB cpu_usec anon_B file_B disk_used_B).
# Rows with a non-numeric field are skipped. Fails without a sample.
bench_range_stats() {
  jq -Rsc '
    def median: sort | length as $n | if $n % 2 == 1 then .[($n - 1) / 2] else (.[$n / 2 - 1] + .[$n / 2]) / 2 end;
    [split("\n")[] | select(test("^[0-9]")) | [splits("[ \t]+")]
      | select(length >= 8 and all(.[:8][]; test("^[0-9]+(\\.[0-9]+)?$"))) | map(tonumber)]
    | select(length > 0)
    | {entry_h: .[0][1], exit_h: .[-1][1], elapsed_s: ((.[-1][0] - .[0][0]) * 10 | round / 10),
       cpu_usec: (.[-1][4] - .[0][4]), samples: length,
       mem_peak_B: (map(.[2]) | max), mem_median_B: (map(.[2]) | median),
       rss_peak_KiB: (map(.[3]) | max), rss_median_KiB: (map(.[3]) | median),
       disk_used_peak_B: (map(.[7]) | max)}' "$1" | grep .
}
