"""Compare all replay RPC fields against an independent mainnet archive."""

import argparse
import hashlib
import json
from urllib.parse import urlencode
from urllib.request import Request, urlopen


# Choose blocks within each mainnet era, away from the upgrade boundary.
# Mainnet moved from app version 6 directly to 8.
SAMPLES = {
    120_000: 1,
    2_500_000: 2,
    3_000_000: 3,
    6_700_000: 4,
    8_218_923: 5,
    9_217_879: 6,
    11_000_000: 8,
    12_000_000: 9,
    15_530_101: 10,
}


def fetch(base, path):
    request = Request(base.rstrip("/") + "/" + path,
                      headers={"User-Agent": "celestia-rs/blockstore-rpc-verifier"})
    with urlopen(request, timeout=60) as response:
        envelope = json.load(response)
    if "error" in envelope or envelope.get("result") is None:
        raise ValueError(f"{path}: RPC returned no successful result")
    return envelope["result"]


def compare(candidate, reference, method, **params):
    path = method + "?" + urlencode(params)
    actual = fetch(candidate, path)
    expected = fetch(reference, path)
    # Ignore whitespace, object key order and the transport's request ID.
    # Compare serialized values so JSON true and 1 remain distinct.
    def canonical(value):
        return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()

    encoded = canonical(actual)
    if encoded != canonical(expected):
        raise ValueError(f"{path}: result differs from reference")
    print(f"{path} {hashlib.sha256(encoded).hexdigest()}", flush=True)
    return actual


def verify(candidate, reference):
    if candidate.rstrip("/") == reference.rstrip("/"):
        raise ValueError("choose two distinct archive endpoints")
    for height, version in SAMPLES.items():
        block = compare(candidate, reference, "block", height=height)["block"]
        header = block["header"]
        if (header["chain_id"], int(header["height"]), int(header["version"]["app"])) != (
            "celestia", height, version
        ):
            raise ValueError(f"{height}: unexpected chain, height or app version")
        if height in (8_218_923, 9_217_879) and not block["evidence"]["evidence"]:
            raise ValueError(f"{height}: expected block evidence")
        compare(candidate, reference, "commit", height=height)
        compare(candidate, reference, "consensus_params", height=height)
        page, seen = 1, 0
        while True:
            result = compare(candidate, reference, "validators", height=height, page=page, per_page=50)
            count, total = int(result["count"]), int(result["total"])
            if int(result["block_height"]) != height or count != len(result["validators"]):
                raise ValueError(f"{height}: invalid validator response")
            if count <= 0 or seen + count > total:
                raise ValueError(f"{height}: invalid validator page {page}")
            seen += count
            if seen == total:
                break
            page += 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate")
    parser.add_argument("reference")
    args = parser.parse_args()
    verify(args.candidate, args.reference)
