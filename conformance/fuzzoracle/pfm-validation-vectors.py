#!/usr/bin/env python3
"""Regenerate PFM validation ACK vectors with both pinned Go oracle binaries."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--v9", type=Path, required=True)
    parser.add_argument("--v10", type=Path, required=True)
    parser.add_argument("--cases", type=Path, default=ROOT / "conformance/fixtures/packetforward/memo-validation-acks.json")
    parser.add_argument("--out", type=Path, required=True, help="new output directory")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    inputs = [{"name": c["name"], "memo": c["memo"]} for c in json.loads(args.cases.read_text())]
    results, binaries = {}, {}
    for version, binary in ((9, args.v9.resolve()), (10, args.v10.resolve())):
        info = subprocess.check_output(["go", "version", "-m", str(binary)], text=True)
        if not info.splitlines()[0].endswith("go1.26.9"):
            raise ValueError(f"oracle v{version} was not built with Go 1.26.9")
        app = "v9.0.8" if version == 9 else "v10.5.0"
        sdk = "v0.52.8" if version == 9 else "v0.52.12"
        required = [f"dep\tgithub.com/celestiaorg/celestia-app/v{version}\t{app}\t",
                    "dep\tgithub.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8\tv8.2.0\t",
                    "=>\tgithub.com/celestiaorg/ibc-go/v8\tv8.7.2\t",
                    f"=>\tgithub.com/celestiaorg/cosmos-sdk\t{sdk}\t"]
        if any(token not in info for token in required):
            raise ValueError(f"oracle v{version} does not match the pinned dependency graph")
        selected, keep = [], False
        for line in info.splitlines()[1:]:
            if "\tdep\t" in line:
                keep = any(name in line for name in ("celestia-app/", "cosmos-sdk\t", "ibc-go/v8\t", "packet-forward-middleware/"))
            if keep or "\tbuild\t" in line:
                selected.append(line.strip())
        binaries[str(version)] = {"sha256": digest(binary), "compiler": "go1.26.9", "build_info": selected}
        with tempfile.TemporaryDirectory(prefix="pfm-oracle-") as home:
            env = {**os.environ, "FUZZ_ORACLE_TMPDIR": home, "CELESTIA_APP_HOME": home}
            wire = "".join("json_pfm_ack " + c["memo"].encode().hex() + "\n" for c in inputs)
            run = subprocess.run([str(binary)], input=wire, text=True, capture_output=True,
                                 check=True, env=env, timeout=60)
        outputs = [json.loads(line) for line in run.stdout.splitlines()]
        if len(outputs) != len(inputs) or any(not o.get("ok") or o.get("panic") for o in outputs):
            raise ValueError(f"incomplete validation outputs at v{version}")
        results[version] = outputs
        records = [{"app_version": version, "op": "json_pfm_ack", "note": "PFM validation ACK: " + c["name"],
                    "input": c["memo"].encode().hex(), "go": o} for c, o in zip(inputs, outputs)]
        (args.out / f"v{version}.jsonl").write_text("".join(json.dumps(r, sort_keys=True, ensure_ascii=True) + "\n" for r in records))
    if results[9] != results[10]:
        raise ValueError("pinned Go versions disagree; do not collapse their callback fixtures")
    cases = []
    for c, o in zip(inputs, results[10]):
        case = {**c, "valid": o["out"] == "valid"}
        if not case["valid"]:
            ack, commitment = o["out"].split()
            if hashlib.sha256(bytes.fromhex(ack)).hexdigest() != commitment:
                raise ValueError("ACK commitment does not match emitted bytes")
            case.update(ack_hex=ack, commitment_hex=commitment)
        cases.append(case)
    fixture = args.out / "memo-validation-acks.json"
    fixture.write_text(json.dumps(cases, ensure_ascii=True, indent=2) + "\n")
    paths = [Path("conformance/fuzzoracle/pfm-validation-vectors.py")]
    paths += [Path(f"conformance/fuzzoracle/{v}/{name}") for v in ("v9", "v10") for name in ("jsondecode.go", "go.mod", "go.sum")]
    receipt = {"schema_version": 1, "vectors_per_version": len(inputs), "binaries": binaries,
               "source_sha256": {str(p): digest(ROOT / p) for p in paths},
               "fixture_sha256": digest(fixture),
               "module_source_commits": {"app_v9": "220ef14eab5f6849c8129633e8bc5bc55995ecfa",
                                         "app_v10": "6013e758d5dceb75827ad191c8d739d7c43cfa04",
                                         "pfm": "e16c798c788097d457144530642b9b3455ab6a4b",
                                         "ibc_go": "a17d4368090a6ab6d97c04a1b3e45c21fd0b7478"},
               "scope": "invalid-metadata OnRecvPacket callback ACK and commitment; no IBC proof lifecycle"}
    (args.out / "provenance.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"recorded {len(cases)} cases at each pinned Go version")


if __name__ == "__main__":
    main()
