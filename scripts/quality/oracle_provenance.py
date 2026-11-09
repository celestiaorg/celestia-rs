#!/usr/bin/env python3
"""Record identities and protobuf-go error-prefix seeds for oracle binaries."""

import argparse
import json
from pathlib import Path
import sys

from qualify import digest, protobuf_go_detrand


def provenance(specifications):
    binaries = {}
    for specification in specifications:
        label, separator, raw_path = specification.partition("=")
        if not separator or not label or not raw_path:
            raise ValueError(f"expected LABEL=PATH, got {specification!r}")
        if label in binaries:
            raise ValueError(f"duplicate binary label {label!r}")
        path = Path(raw_path).resolve(strict=True)
        if not path.is_file():
            raise ValueError(f"binary is not a file: {path}")
        binaries[label] = {
            "path": raw_path,
            "size": path.stat().st_size,
            "sha256": digest(path),
            "protobuf_go_detrand": protobuf_go_detrand(path),
        }
    return {
        "schema_version": 1,
        "scope": "exact binary bytes; this receipt does not establish qualification",
        "binaries": binaries,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", action="append", default=[], metavar="LABEL=PATH",
                        help="record one named binary (repeatable)")
    args = parser.parse_args()
    if not args.binary:
        parser.error("supply at least one --binary LABEL=PATH")
    try:
        receipt = provenance(args.binary)
        with args.out.open("x") as output:
            json.dump(receipt, output, indent=2, sort_keys=True)
            output.write("\n")
    except (OSError, ValueError) as error:
        parser.exit(2, f"oracle provenance: {error}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
