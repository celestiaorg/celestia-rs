#!/usr/bin/env bash
# Tests for node-metrics.sh with curl, ps, du and the celestia-rs CLI
# stubbed on PATH. Run: packaging/macos/monitoring/test-node-metrics.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/out" "$t/val/config" "$t/val/data" "$t/full/config" "$t/full/data"
fails=0

# --- stubs -----------------------------------------------------------------
cat >"$t/bin/curl" <<'EOF'
#!/usr/bin/env bash
url=${!#}
case $url in
  http://127.0.0.1:1111/status) cat <<J
{"result":{"sync_info":{"latest_block_height":"1830400","latest_block_time":"2026-10-19T14:26:41.17840124Z","catching_up":false}}}
J
  ;;
  http://127.0.0.1:2222/status) cat <<J
{"result":{"sync_info":{"latest_block_height":"15099989","latest_block_time":"2026-10-19T14:26:40Z","catching_up":true}}}
J
  ;;
  *) exit 7 ;;
esac
EOF
cat >"$t/bin/ps" <<EOF
#!/usr/bin/env bash
cat <<P
  101  1933696 1-02:03:04.50 /opt/x/celestia-rs start --home $t/val
  202   349008      01:02.25 /opt/x/celestia-rs-mainnet start --home $t/full
  303     1000      00:00.01 /bin/zsh
P
EOF
cat >"$t/bin/du" <<'EOF'
#!/usr/bin/env bash
echo "2048	$2"
EOF
# celestia-rs: the validator home has signing info, the full node has none.
cat >"$t/bin/celestia-rs" <<EOF
#!/usr/bin/env bash
home=\${!#}
case "\$*" in
  "comet show-address --home $t/val") echo celestiavalcons1val ;;
  "comet show-address --home $t/full") echo celestiavalcons1full ;;
  "query slashing signing-info celestiavalcons1val --home $t/val")
    echo '{"val_signing_info":{"missed_blocks_counter":"184","jailed_until":"2999-01-01T00:00:00Z","tombstoned":false}}' ;;
  "query slashing signing-info celestiavalcons1full --home $t/full")
    echo 'Error: rpc error: code = NotFound' >&2; exit 1 ;;
  "query slashing params --home "*)
    echo '{"params":{"signed_blocks_window":"10000","min_signed_per_window":"0.001000000000000000"}}' ;;
  *) echo "unexpected: \$*" >&2; exit 2 ;;
esac
EOF
chmod +x "$t/bin/"*

cat >"$t/nodes.conf" <<EOF
# network home rpc reference-rpc
mocha   $t/val  http://127.0.0.1:1111 https://ref.invalid
mainnet $t/full http://127.0.0.1:2222 https://ref.invalid
down    $t/none http://127.0.0.1:3333 https://ref.invalid
EOF

expect() { # expect <file> <exact line>
  if ! grep -qxF -- "$2" "$1"; then
    echo "FAIL: missing line: $2"; fails=$((fails + 1))
  fi
}

# --- collect -----------------------------------------------------------------
PATH="$t/bin:$PATH" CELESTIA_RS_BIN="$t/bin/celestia-rs" \
  "$here/node-metrics.sh" collect "$t/nodes.conf" "$t/out"
f=$t/out/celestia_rs.prom
[[ -f $f ]] || { echo "FAIL: $f not written"; exit 1; }

expect "$f" 'celestia_rs_rpc_up{network="mocha"} 1'
expect "$f" 'celestia_rs_catching_up{network="mocha"} 0'
expect "$f" 'celestia_rs_catching_up{network="mainnet"} 1'
expect "$f" 'celestia_rs_latest_block_height{network="mocha"} 1830400'
expect "$f" 'celestia_rs_latest_block_time_seconds{network="mocha"} 1792420001'
expect "$f" 'celestia_rs_process_up{network="mocha"} 1'
expect "$f" 'celestia_rs_process_resident_memory_bytes{network="mocha"} 1980104704'
expect "$f" 'celestia_rs_process_cpu_seconds_total{network="mocha"} 93784.50'
expect "$f" 'celestia_rs_process_cpu_seconds_total{network="mainnet"} 62.25'
expect "$f" 'celestia_rs_data_dir_bytes{network="mocha"} 2097152'
expect "$f" 'celestia_rs_validator_signing_info_found{network="mocha"} 1'
expect "$f" 'celestia_rs_validator_missed_blocks_window{network="mocha"} 184'
expect "$f" 'celestia_rs_validator_signed_blocks_window{network="mocha"} 10000'
expect "$f" 'celestia_rs_validator_min_signed_per_window{network="mocha"} 0.001000000000000000'
expect "$f" 'celestia_rs_validator_jailed{network="mocha"} 1'
expect "$f" 'celestia_rs_validator_tombstoned{network="mocha"} 0'
expect "$f" 'celestia_rs_validator_signing_info_found{network="mainnet"} 0'
expect "$f" 'celestia_rs_rpc_up{network="down"} 0'
expect "$f" 'celestia_rs_process_up{network="down"} 0'
if grep -q 'validator_missed_blocks_window{network="mainnet"}' "$f"; then
  echo "FAIL: full node must not report missed blocks"; fails=$((fails + 1))
fi

# --- apphash -----------------------------------------------------------------
cat >"$t/check-apphash.sh" <<'EOF'
#!/usr/bin/env bash
case $1 in
  http://127.0.0.1:1111) echo OK; exit 0 ;;
  http://127.0.0.1:2222) echo MISMATCH >&2; exit 1 ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$t/check-apphash.sh"
PATH="$t/bin:$PATH" CHECK_APPHASH="$t/check-apphash.sh" \
  "$here/node-metrics.sh" apphash "$t/nodes.conf" "$t/out"
a=$t/out/celestia_rs_apphash.prom
[[ -f $a ]] || { echo "FAIL: $a not written"; exit 1; }
expect "$a" 'celestia_rs_apphash_match{network="mocha"} 1'
expect "$a" 'celestia_rs_apphash_match{network="mainnet"} 0'
expect "$a" 'celestia_rs_apphash_match{network="down"} -1'
grep -q '^celestia_rs_apphash_check_timestamp_seconds{network="mocha"} [0-9]' "$a" \
  || { echo "FAIL: missing apphash timestamp"; fails=$((fails + 1)); }

if ((fails)); then echo "$fails failure(s)"; exit 1; fi
echo "all node-metrics tests passed"
