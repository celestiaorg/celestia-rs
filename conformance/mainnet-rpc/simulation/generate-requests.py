#!/usr/bin/env python3
"""Create deterministic decoder, ante and execution Simulate requests."""
import argparse
import json
from pathlib import Path


def varint(value):
    out = bytearray()
    while value > 127:
        out.append(value & 127 | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def message(field, value):
    if isinstance(value, str):
        value = value.encode()
    return varint(field * 8 + 2) + varint(len(value)) + value


def integer(field, value):
    return varint(field * 8) + varint(value)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--genesis", required=True, type=Path)
parser.add_argument("--out", required=True, type=Path)
args = parser.parse_args()
genesis = json.loads(args.genesis.read_text())
accounts = genesis["app_state"]["auth"]["accounts"]
sender, receiver = accounts[1]["address"], accounts[0]["address"]


def tx(amount="1", sequence=0, memo="", signatures=True):
    coin = message(1, "utia") + message(2, amount)
    send = message(1, sender) + message(2, receiver) + message(3, coin)
    any_msg = message(1, "/cosmos.bank.v1beta1.MsgSend") + message(2, send)
    body = message(1, any_msg)
    if memo:
        body += message(2, memo)
    mode = message(1, integer(1, 1))
    signer = message(2, mode)
    if sequence:
        signer += integer(3, sequence)
    auth = message(1, signer) + message(2, integer(2, 200000))
    raw = message(1, body) + message(2, auth)
    if signatures:
        raw += message(3, bytes(64))
    return message(2, raw)


cases = [
    ("missing-tx-bytes", "request-validation", b""),
    ("malformed-request", "transport-decoder", b"\xff"),
    ("decoder-wrong-wire-type", "transaction-decoder", message(2, b"abc")),
    ("decoder-truncated-body", "transaction-decoder", message(2, b"\x0a\x03a")),
    ("decoder-empty-tx", "transaction-validation", message(1, b"")),
    ("ante-missing-signatures", "ante", tx(signatures=False)),
    ("ante-wrong-sequence", "ante", tx(sequence=999)),
    ("ante-long-memo", "ante", tx(memo="m" * 257)),
    ("execution-insufficient-funds", "execution", tx(amount="2000000000000")),
    ("successful-simulation", "success-control", tx()),
    ("successful-simulation-repeat", "success-control", tx()),
]
requests = [{"name": name, "phase": phase, "request_hex": body.hex(), "height": "20"}
            for name, phase, body in cases]
for name, height in (("missing-latest-height", ""), ("missing-historical-height", "10"), ("future-height", "21")):
    requests.append({"name": name, "phase": "height-metadata", "request_hex": "", "height": height})
args.out.write_text(json.dumps(requests, indent=2) + "\n")
