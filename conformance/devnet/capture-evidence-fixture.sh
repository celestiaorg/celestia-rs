#!/usr/bin/env bash
# Called by double-sign-evidence.sh only after the mixed network has passed
# its chain/slashing checks and stopped. Never copies keys or node databases.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
NODE=$1 EV_H=$2 V1_ADDR=$3 OUT=$4
cp "$NODE/config/genesis.json" "$OUT/genesis.json"
"$BIN/rpc-recorder" dump -data "$NODE/data" -through "$EV_H" -out "$OUT/blocks.bin.gz"
"$BIN/rpc-recorder" validation-state -data "$NODE/data" -height "$EV_H" -out "$OUT/validation.json"
go version -m "$BIN/celestia-appd" "$BIN/rpc-recorder" | sed 's/[[:space:]]*$//' > "$OUT/go-build-info.txt"
python3 - "$ROOT" "$NODE" "$EV_H" "$V1_ADDR" "$OUT" "$BIN" <<'PY'
import hashlib, json, os, pathlib, re, subprocess, sys
root, node, height, addr, out, bin_dir = sys.argv[1:]
out = pathlib.Path(out)
info = (out / "go-build-info.txt").read_text()
# Reject a pre-existing DEVNET_BIN built against different module sources.
# build-bins.sh builds a copied module archive, so its main module is
# marked (devel). Its successful-build stamp identifies the pinned source.
assert re.search(r'\bgithub.com/celestiaorg/celestia-app/v10\s+\(devel\)', info), info
assert pathlib.Path(bin_dir, "celestia-appd.module").read_text().strip() == "github.com/celestiaorg/celestia-app/v10@v10.5.0"
assert re.search(r'\bgithub.com/celestiaorg/celestia-core\s+v0\.42\.3\b', info), info
toolchains = re.findall(r': (go[0-9.]+)\n', info)
assert toolchains == ["go1.26.9", "go1.26.9"], toolchains
stats = json.loads(pathlib.Path(node).parent.joinpath("stats-E.json").read_text())
evs = dict(stats["evidence"])[int(height)]
assert evs
value = evs[0].get("value", evs[0])
for ev in evs:
    proof = ev.get("value", ev)
    assert ev["type"].endswith("DuplicateVoteEvidence")
    assert proof["vote_a"]["validator_address"].upper() == addr
    assert proof["vote_a"]["height"] == value["vote_a"]["height"]
meta = {
    "chain_id": json.loads((out / "genesis.json").read_text())["chain_id"],
    "evidence_height": int(height), "validator_address": addr,
    "evidence_count": len(evs),
    "evidence_vote_height": int(value["vote_a"]["height"]),
    "sources": {"go": "go1.26.9", "celestia_app": "v10.5.0", "celestia_core": "v0.42.3",
                "celestia_rs": subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()},
    "regenerate": "RUST_LOG=info EVIDENCE_FIXTURE_OUT=/tmp/full-block-evidence conformance/devnet/double-sign-evidence.sh",
    "sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.iterdir())},
}
meta["sources"]["celestia_rs_features"] = "no-default-features" if os.environ.get("DEVNET_NO_DEFAULT_FEATURES") == "1" else "default"
if meta["sources"]["celestia_rs_features"] == "no-default-features":
    meta["regenerate"] = "DEVNET_NO_DEFAULT_FEATURES=1 " + meta["regenerate"]
for label, module, version in [
    ("celestia_app", "github.com/celestiaorg/celestia-app/v10", "v10.5.0"),
    ("celestia_core", "github.com/celestiaorg/celestia-core", "v0.42.3"),
]:
    source = json.loads(subprocess.check_output(
        ["go", "mod", "download", "-json", f"{module}@{version}"],
        cwd=pathlib.Path(root) / "conformance/rpccorpus/recorder", text=True))
    meta["sources"][label + "_commit"] = source["Origin"]["Hash"]
    meta["sources"][label + "_sum"] = source["Sum"]
    if label == "celestia_core":
        assert source["Sum"] in info, f"binary provenance missing {module} {source['Sum']}"
meta["capture_sources_sha256"] = {
    name: hashlib.sha256(pathlib.Path(root, name).read_bytes()).hexdigest()
    for name in ["crates/tier-4-runtime/api-server/Cargo.toml",
                 "crates/tier-5-node/celestia-rs/Cargo.toml",
                 "conformance/devnet/build-bins.sh",
                 "conformance/devnet/double-sign-evidence.sh",
                 "conformance/devnet/capture-evidence-fixture.sh",
                 "conformance/devnet/lib-mixed.sh",
                 "conformance/rpccorpus/recorder/main.go",
                 "conformance/rpccorpus/recorder/validation.go"]
}
meta["celestia_rs_binary_sha256"] = hashlib.sha256(pathlib.Path(root, "target/debug/celestia-rs").read_bytes()).hexdigest()
meta["celestia_app_binary_sha256"] = hashlib.sha256(pathlib.Path(bin_dir, "celestia-appd").read_bytes()).hexdigest()
validation = json.loads((out / "validation.json").read_text())
meta["block_hash"] = validation["block_hash"]
meta["evidence_hash"] = validation["evidence_hash"]
(out / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
PY
