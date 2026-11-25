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
BENCH_SEEDS=8542cd7e6bf9d260fef543bc49e59be5a3fa9074@seed.publicnode.com:26656,ade4d8bc8cbe014af6ebdf3cb7b1e9ad36f412c0@seeds.polkachu.com:11656
BENCH_PEERS=076c9cc9de3039819a98522aeea39e8c72b49682@peer.celestia.mainnet.dteam.tech:28656,8dcecfd3fb44990af00408bbd70ab141c33d4513@67.213.115.125:36656

# bench_valid KIND VALUE: strict format check for every external input.
bench_valid() {
  local peer='[0-9a-f]{40}@[A-Za-z0-9.-]{1,253}:[0-9]{1,5}'
  case $1 in
    campaign) [[ $2 =~ ^(genesis-prefix|state-sync|bridge)$ ]] ;;
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

# bench_pair_check SNAP_A SNAP_B: a pair is valid only if both sides
# restored the same snapshot height.
bench_pair_check() {
  if [[ ! $1 =~ ^[0-9]+$ || ! $2 =~ ^[0-9]+$ ]]; then echo discard:missing_snapshot; return 1; fi
  if [[ $1 != "$2" ]]; then echo discard:different_snapshots; return 1; fi
  echo valid
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
# at most 3000 later lines about state sync, snapshots, the light client,
# block sync or problems. A whole node log is too large to publish.
bench_log_excerpt() {
  awk 'NR <= 200 { print; next }
       tolower($0) ~ /snapshot|statesync|state sync|light|blocksync|block sync|chunk|error|warn|panic|fatal/ { keep[n++] = $0 }
       END { for (i = (n > 3000 ? n - 3000 : 0); i < n; i++) print keep[i] }'
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
