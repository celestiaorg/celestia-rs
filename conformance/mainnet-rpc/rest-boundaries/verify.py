#!/usr/bin/env python3
"""Verify the frozen REST evidence and its local provenance files."""

import hashlib
import json
import pathlib


HERE = pathlib.Path(__file__).parent / "v10.5.0"


def digest(name: str) -> str:
    return hashlib.sha256((HERE / name).read_bytes()).hexdigest()


def main() -> None:
    provenance = json.loads((HERE / "provenance.json").read_text())
    evidence = provenance["evidence"]
    expected = {
        "rest-heights.json": evidence["capture_sha256"],
        "capture-source.py": evidence["capture_source_sha256"],
        "go-state.json": evidence["go_state_sha256"],
        "mainnet-observation.json": evidence["mainnet_observation_sha256"],
        "oracle-build.json": provenance["oracle"]["build_recipe_sha256"],
    }
    mismatches = [name for name, wanted in expected.items() if digest(name) != wanted]
    if mismatches:
        raise SystemExit(f"provenance hash mismatch: {', '.join(mismatches)}")
    capture = json.loads((HERE / "rest-heights.json").read_text())
    if len(capture["cases"]) != 42:
        raise SystemExit("expected 42 REST cases")
    build = json.loads((HERE / "oracle-build.json").read_text())
    for key in ("binary_sha256", "compiler", "module", "source_commit"):
        if build[key] != provenance["oracle"][key]:
            raise SystemExit(f"oracle {key} does not match the build recipe")


if __name__ == "__main__":
    main()
