"""Minimal Cosmos SDK tx building for the soak load generator: secp256k1
keys (via `cryptography`), bech32 addresses, hand-encoded protobuf for
`MsgSend` txs signed with SIGN_MODE_DIRECT, and `QueryAccountResponse`
decoding. No celestia-app code is involved, so the load is an independent
client of both the Go and the Rust nodes."""

import base64
import hashlib
import json
import urllib.request

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141

# ---------------------------------------------------------------- bech32
CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _polymod(values):
    gen = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for v in values:
        b = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ v
        for i in range(5):
            chk ^= gen[i] if ((b >> i) & 1) else 0
    return chk


def _hrp_expand(hrp):
    return [ord(x) >> 5 for x in hrp] + [0] + [ord(x) & 31 for x in hrp]


def _convertbits(data, frombits, tobits):
    acc, bits, ret, maxv = 0, 0, [], (1 << tobits) - 1
    for value in data:
        acc = (acc << frombits) | value
        bits += frombits
        while bits >= tobits:
            bits -= tobits
            ret.append((acc >> bits) & maxv)
    if bits:
        ret.append((acc << (tobits - bits)) & maxv)
    return ret


def bech32(hrp, data):
    d = _convertbits(data, 8, 5)
    poly = _polymod(_hrp_expand(hrp) + d + [0] * 6) ^ 1
    chk = [(poly >> 5 * (5 - i)) & 31 for i in range(6)]
    return hrp + "1" + "".join(CHARSET[x] for x in d + chk)


# ---------------------------------------------------------------- protobuf
def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def fbytes(num, b):
    if isinstance(b, str):
        b = b.encode()
    if not b:
        return b""
    return varint(num << 3 | 2) + varint(len(b)) + b


def fvarint(num, v):
    return varint(num << 3) + varint(v) if v else b""


def any_(type_url, value):
    return fbytes(1, type_url) + fbytes(2, value)


def coin(denom, amount):
    return fbytes(1, denom) + fbytes(2, str(amount))


def decode(buf):
    """Field number -> list of raw values (ints for varints, bytes else)."""
    out, i = {}, 0
    while i < len(buf):
        key, i = _read_varint(buf, i)
        num, wt = key >> 3, key & 7
        if wt == 0:
            v, i = _read_varint(buf, i)
        elif wt == 2:
            n, i = _read_varint(buf, i)
            v, i = buf[i : i + n], i + n
        elif wt == 1:
            v, i = buf[i : i + 8], i + 8
        elif wt == 5:
            v, i = buf[i : i + 4], i + 4
        else:
            raise ValueError(f"wire type {wt}")
        out.setdefault(num, []).append(v)
    return out


def _read_varint(buf, i):
    shift = result = 0
    while True:
        b = buf[i]
        i += 1
        result |= (b & 0x7F) << shift
        if not b & 0x80:
            return result, i
        shift += 7


# ---------------------------------------------------------------- keys / txs
class Key:
    def __init__(self, secret: int):
        self.sk = ec.derive_private_key(secret, ec.SECP256K1())
        self.pub = self.sk.public_key().public_bytes(
            serialization.Encoding.X962, serialization.PublicFormat.CompressedPoint
        )
        h = hashlib.new("ripemd160", hashlib.sha256(self.pub).digest()).digest()
        self.address = bech32("celestia", h)

    @classmethod
    def from_seed(cls, seed: str):
        return cls(int.from_bytes(hashlib.sha256(seed.encode()).digest(), "big") % (N - 1) + 1)

    def sign(self, msg: bytes) -> bytes:
        r, s = decode_dss_signature(self.sk.sign(msg, ec.ECDSA(hashes.SHA256())))
        if s > N // 2:
            s = N - s
        return r.to_bytes(32, "big") + s.to_bytes(32, "big")


def msg_send(frm, to, amount, denom="utia"):
    return any_(
        "/cosmos.bank.v1beta1.MsgSend", fbytes(1, frm) + fbytes(2, to) + fbytes(3, coin(denom, amount))
    )


def sign_tx(key: Key, chain_id, account_number, sequence, msgs, gas, fee, memo=""):
    body = b"".join(fbytes(1, m) for m in msgs) + fbytes(2, memo)
    pk = any_("/cosmos.crypto.secp256k1.PubKey", fbytes(1, key.pub))
    signer = fbytes(1, pk) + fbytes(2, fbytes(1, fvarint(1, 1))) + fvarint(3, sequence)
    auth = fbytes(1, signer) + fbytes(2, fbytes(1, coin("utia", fee)) + fvarint(2, gas))
    doc = fbytes(1, body) + fbytes(2, auth) + fbytes(3, chain_id) + fvarint(4, account_number)
    return fbytes(1, body) + fbytes(2, auth) + fbytes(3, key.sign(doc))


# ---------------------------------------------------------------- RPC
def rpc_call(url, method, params, timeout=10):
    req = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers={"Content-Type": "application/json"},
    )
    d = json.load(urllib.request.urlopen(req, timeout=timeout))
    if "error" in d and d["error"]:
        raise RuntimeError(str(d["error"])[:300])
    return d["result"]


def broadcast(url, tx: bytes, timeout=10):
    return rpc_call(url, "broadcast_tx_sync", {"tx": base64.b64encode(tx).decode()}, timeout)


def query_account(url, address):
    """(account_number, sequence) of `address`, or None if unknown."""
    r = rpc_call(
        url,
        "abci_query",
        {"path": "/cosmos.auth.v1beta1.Query/Account", "data": fbytes(1, address).hex()},
    )["response"]
    if r.get("code", 0) != 0 or not r.get("value"):
        return None
    resp = decode(base64.b64decode(r["value"]))
    anyv = decode(resp[1][0])
    acc = decode(anyv[2][0])
    return acc.get(3, [0])[0], acc.get(4, [0])[0]
