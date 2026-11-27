#!/bin/bash
# Offline check of archive-export.sh: batching by exporter version (scan-ahead,
# PARALLEL at a time), per-height failure isolation, metadata uploaded last,
# and done heights skipped on rerun. Fake exporter, RPC and bucket client.
#   bash conformance/archive-export/tools/archive-export-test.sh
set -u
W=$(mktemp -d); trap 'rm -rf $W' EXIT
SRC=$(cd "$(dirname "$0")" && pwd)/archive-export.sh
mkdir -p $W/bin $W/tools $W/rs/.git $W/rs/conformance/mainnet-rpc $W/rs/conformance/fixtures/mainnet $W/home/data/application.db
echo '{"windows":[]}' > $W/rs/conformance/fixtures/mainnet/replay-windows.json
printf 'import sys\nprint("Verified")\n' > $W/rs/conformance/mainnet-rpc/verify-replay-checkpoint.py
cat > $W/bin/vmx-gcs <<'EOF'
#!/bin/bash
echo "$*" >> "$UPLOADS"
EOF
cat > $W/bin/curl <<'EOF'
#!/bin/bash
echo '{"result":{"block":{"header":{"app_hash":"AA"}},"block_id":{"hash":"BB"}}}'
EOF
for v in v046 v050; do F=$([ $v = v046 ] && echo 2 || echo 3); cat > $W/tools/archive-export-$v <<EOF
#!/bin/bash
# fake exporter: -height list -parallel N -out tmpl -log tmpl
while ((\$#)); do case \$1 in -height) hs=\$2; shift 2;; -out) o=\$2; shift 2;; -log) l=\$2; shift 2;; -parallel) p=\$2; shift 2;; *) shift;; esac; done
echo "\$hs p=\$p" >> "$W/calls-$v"
for h in \${hs//,/ }; do
  out=\${o//%d/\$h}; log=\${l//%d/\$h}; mkdir -p \$out/\$h/$F \$(dirname \$log)
  [ \$h = 999 ] && { echo "error: boom" > \$log; continue; }
  # 8 finishes later than 7: record whether 7 was already uploaded by then.
  if [ \$h = 8 ]; then
    for i in \$(seq 50); do grep -q 'go-snapshots/7/2/metadata.json' "\$UPLOADS" && break; sleep 0.2; done
    grep -q 'go-snapshots/7/2/metadata.json' "\$UPLOADS" && echo early > "$W/seven" || echo late > "$W/seven"
  fi
  echo chunk > \$out/\$h/$F/0; echo "Snapshot created at height \$h" > \$log
done
EOF
chmod +x $W/tools/archive-export-$v; done
chmod +x $W/bin/*
sed -e "s|^OUT=/data/ckpt; T=/data/tools; RS=/data/celestia-rs|OUT=$W/ckpt; T=$W/tools; RS=$W/rs|" -e "s|^HOME_DATA=/data/home/data|HOME_DATA=$W/home/data|" \
  -e 's|^git -C $RS fetch.*||' -e 's|^  { echo "cannot update $RS to origin/main"; exit 1; }||' $SRC > $W/ae.sh
export UPLOADS=$W/uploads
PATH=$W/bin:$PATH HEIGHTS="1:v046 2:v046 3:v050 4:v046 999:v046 5:v046 6:v050" PARALLEL=3 bash $W/ae.sh > $W/out.txt 2>&1
fail=0
bash -n $SRC || { echo "FAIL syntax"; fail=1; }
[ "$(cat $W/calls-v046)" = $'1,2,4 p=3\n999,5 p=2' ] || { echo "FAIL v046 batches: $(cat $W/calls-v046)"; fail=1; }
[ "$(cat $W/calls-v050)" = "3,6 p=2" ] || { echo "FAIL v050 batches: $(cat $W/calls-v050)"; fail=1; }
for h in 1 2 4 5; do [ -s $W/ckpt/$h-v046/checkpoint.json ] || { echo "FAIL no checkpoint for $h"; fail=1; }; done
[ -e $W/ckpt/999-v046/checkpoint.json ] && { echo "FAIL 999 has a checkpoint"; fail=1; }
grep -q "export 999 v046 failed" $W/out.txt || { echo "FAIL 999 failure not reported"; fail=1; }
grep -q 'go-snapshots/1/2/metadata.json' $UPLOADS || { echo "FAIL no metadata upload"; fail=1; }
last1=$(grep -n 'go-snapshots/1/' $UPLOADS | tail -1); echo "$last1" | grep -q metadata.json || { echo "FAIL metadata not last for 1: $last1"; fail=1; }
# rerun skips done heights
: > $W/calls-v046; PATH=$W/bin:$PATH HEIGHTS="1:v046 2:v046 999:v046" PARALLEL=3 bash $W/ae.sh > $W/out2.txt 2>&1
[ "$(cat $W/calls-v046)" = "999 p=1" ] || { echo "FAIL rerun: $(cat $W/calls-v046)"; fail=1; }
# a height is verified and uploaded as soon as its own export completes
: > $UPLOADS; PATH=$W/bin:$PATH HEIGHTS="7:v046 8:v046" PARALLEL=2 POLL_SECS=0.2 bash $W/ae.sh > $W/out3.txt 2>&1
[ "$(cat $W/seven 2>/dev/null)" = early ] || { echo "FAIL 7 not uploaded while 8 was still exporting: $(cat $W/seven 2>/dev/null)"; fail=1; }
for h in 7 8; do [ "$(grep -c "go-snapshots/$h/2/metadata.json" $UPLOADS)" = 1 ] || { echo "FAIL $h metadata uploaded $(grep -c "go-snapshots/$h/2/metadata.json" $UPLOADS) times"; fail=1; }; done
[ $fail = 0 ] && echo "archive-export batching: all checks passed" || { cat $W/out.txt; exit 1; }
