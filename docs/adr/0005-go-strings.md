# ADR 0005 — Go strings: invalid UTF-8 in protobuf `string` fields

**Status:** Accepted (2026-10-15).

## Context

A Go `string` is a byte sequence. celestia-app decodes every message with
gogoproto, whose generated `Unmarshal` copies a `string` field's bytes
without a UTF-8 check (no `.pb.go` of cosmos-sdk v0.52.12 / v0.52.8,
ibc-go v8.7.2, hyperlane-cosmos v1.3.0 / v1.1.0, celestia-app v10.4.0 /
v9.0.8 or celestia-core v0.42.3 has one). The handlers then store, emit
and log those bytes. prost decodes a `string` into a Rust `String` and
rejects invalid UTF-8, so the Rust router failed (`tx parse error`) every
message Go executes with such a string: a validator moniker, a proposal
title, a vote's metadata, a memo. Any user could split the network with
one cheap tx (security.md, fuzzing finding 11).

### What Go does, per path (verified in the module cache)

| Path | Go behavior |
|---|---|
| tx decoding (`DefaultTxDecoder`, `unknownproto`) | no UTF-8 check; body, auth info and the `Any` cached values (`UnpackInterfaces`) keep the bytes |
| `GetSigners` (x/tx `anyutil.Unpack`) | pulsar types (the 116 cosmos-sdk types in `protoregistry.GlobalTypes`) keep the bytes; `dynamicpb` types reject invalid UTF-8 in proto3 string fields — already ported (`getsigners_schema.rs`, finding 1). Applies to top-level messages, `MsgExec` inner messages (authz `DispatchActions`), gov proposal messages (`SubmitProposal`) and ICA host messages (`authenticateTx`). Contents of other `Any` fields (authorizations, allowances, evidence, legacy content, IBC client states / headers) are never checked |
| validation (`ValidateBasic`, msg servers) | `len(s)` counts bytes; parsers (bech32, denoms, decimals) see the bytes; `strings.TrimSpace` / comparisons on bytes; `strings.EqualFold` reads an invalid byte as `RuneError` |
| state | gogoproto `Marshal` writes the bytes; store keys built from strings (`upgrade` done keys, authz grant keys, circuit disable list) carry them |
| events | `sdk.NewAttribute(k, v)` keeps the bytes; typed events (`TypedEventToEvent`: gogoproto `jsonpb`, then `json.RawMessage`) write each invalid byte as `�` |
| tx result | `Log` is the error string with the bytes; `Data` (`TxMsgData`) is protobuf; only code / data / gas enter `LastResultsHash` |
| ABCI wire | gogoproto, bytes as they are |
| JSON decoding (`json.Unmarshal`, gogoproto `jsonpb.Unmarshal`: gov metadata, IBC packet data, ICA `proto3json`, acknowledgements) | invalid UTF-8 inside a JSON string becomes U+FFFD; elsewhere it is a syntax error |
| JSON encoding (`json.Marshal`, `jsonpb`, x/tx `aminojson`) | each invalid byte becomes the six characters `�`; after a `MustSortJSON` round trip it becomes U+FFFD itself |
| `SIGN_MODE_LEGACY_AMINO_JSON` | sign bytes from x/tx `aminojson` over the protobuf-go message: `�` per invalid byte — the signature covers them |
| CometBFT RPC JSON | `encoding/json`: `�` |

## Decision

Keep `String` everywhere and carry a Go string's bytes in it losslessly
(`crates/tier-0-primitives/gostr`):

- **Go form.** `E(bytes)` keeps every valid char except the 128 chars
  U+10FF80..=U+10FFFF (reserved set R: the end of plane 16, private use and
  two noncharacters); every invalid byte, and each byte of the UTF-8 of a
  char in R, becomes the R char `U+10FF00 + byte`. `D` maps R chars back
  to bytes. `D(E(b)) = b` for all bytes, `E(s) = s` for every valid string
  without R chars, and `E` is injective, so string equality is Go's.
- **Codec.** The generated crates (`celestia-proto`, `celestia-core-proto`)
  depend on `prost-gostr` *under the name `prost`*: it re-exports prost
  and replaces only `prost::encoding::string`, which the derive macro calls
  for every `string` field (scalar, repeated, map, oneof). Decoding never
  fails on UTF-8 and yields the Go form; encoding and `encoded_len` write
  `D`. Generated code, types and every caller are unchanged. Other prost
  crates (go-square types: protobuf-go, which does check UTF-8) keep prost.
- **Where the Go form differs from the bytes**, code uses `gostr`:
  `go_len` for Go's `len` (staking description, gov metadata / summary /
  v1beta1 title / description, memo, IBC identifiers and chain IDs),
  `to_go_bytes` / `from_go_bytes` for store keys and raw values built from
  or read into strings (bank denoms, upgrade names, authz message types,
  circuit type URLs, gov constitution, ack and packet JSON), `runes`
  (Go's `for range`) in every Go-JSON string encoder (`�`),
  `json_escape_invalid` for serde-built typed events, `lossy` before
  parsing JSON from a Go string (gov metadata, packet data, ICA, acks),
  `equal_fold` (Go's `strings.EqualFold` with Go 1.26.6's
  `unicode.SimpleFold` table) for authz / feegrant grantee checks, and
  bech32 decoding checks the bytes (Go's errors report the byte and the
  byte length).
- **Output boundaries.** ABCI responses (events, log, data) go through the
  codec. The REST gateway body is `gostr::json_bytes` (Go's `�`);
  RPC JSON quoting writes `�`.
- **`SIGN_MODE_LEGACY_AMINO_JSON`.** The message bytes are re-encoded with
  their strings in Go form before `prost-reflect` decodes them, and the
  JSON writer emits `�`, so the sign bytes are Go's.

Alternatives rejected: a `GoString(Vec<u8>)` field type (prost's `bytes`
adapter is sealed, the derive hardcodes `String`, and ~1,000 fields with
62 k lines of callers would change), `bytes`-typed fields (same churn,
serde changes), and lossy decoding (wrong lengths, gas and stored bytes).

## Consequences

- Invalid UTF-8 executes in Rust exactly where Go executes it; the only
  costs are an R-byte scan on encode and a UTF-8 check on decode (as
  before).
- A Go form must not be turned into bytes with `as_bytes` where Go uses the
  bytes; reviewers check new `as_bytes()` / `len()` on message strings.
- Order between strings with invalid bytes differs from Go's byte order;
  no state machine path sorts such strings (denoms, addresses and IDs are
  validated first).
- JSON input outside the listed paths (genesis) is read as UTF-8 text; a
  genesis char in R would be read as bytes (not reachable from txs).

Nested messages: Go's `GetMsgV1Signers` (authz `MsgExec`, gov proposals,
ICA) re-packs a message and unpacks it with the same `dynamicpb` checks as
the ante handler, so those checks (`getsigners_schema`, now in
`celestia-proto`) run there too (`signers::get_signers_go`,
`txdecode::gogo_repack`).

## Verification

- `utf8` / `v9-utf8` devnet scenarios (`workloads/utf8.sh`, txs built by
  `fuzz-diff craft`): Go v10.4.0 / v9.0.8 execute 17 txs with invalid
  UTF-8 (incl. two signed with `SIGN_MODE_LEGACY_AMINO_JSON`, whose sign
  bytes carry `\ufffd`); the Rust replay matches app hashes, tx results,
  CheckTx, proposals, and logs and events byte for byte; the `utf8` query
  corpus (ABCI and REST) matches.
- `fuzz-diff exec`: Simulate differential over 12 scenarios (v10 and v9),
  ~8 000 mutated txs, ~5 700 of them with invalid UTF-8: no code, gas or
  result difference (security.md, findings 11–16).
