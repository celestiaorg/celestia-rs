# Changelog

## Unreleased

### Added

- Added the Tier-0 `encoding` crate with protobuf, varint, hex, base64, and
  Celestia bech32 helpers plus Go-generated conformance fixtures.
- Added the Tier-0 `crypto` crate spec and updated the M1 index after the
  `encoding` merge.
- Added the Tier-0 `merkle` crate spec so both post-`encoding` parallel crates
  are ready for implementation.
- Added the Tier-0 `merkle` crate with CometBFT-compatible simple Merkle roots,
  inclusion proofs, malformed proof checks, and Go-generated conformance
  fixtures.
- Added the Tier-0 `crypto` crate with CometBFT Ed25519, Cosmos SDK
  secp256k1, and `blst` BLS12-381 helpers plus Go-generated conformance
  fixtures.
- Added the Tier-1 `tendermint-types` crate spec for CometBFT headers,
  commits, validator sets, votes, proposals, canonical sign bytes, and
  fixture requirements.
- Started the Tier-1 `tendermint-types` crate with domain types, protobuf
  conversion, header hashing, canonical sign-byte helpers, and CometBFT
  header conformance fixtures.
- Expanded the Tier-1 `tendermint-types` crate with validator, commit,
  canonical sign-byte, malformed protobuf conformance fixtures, stricter
  validation, and coverage above the M1 threshold.
- Added the Tier-1 `cosmos-types` crate spec for Cosmos SDK transaction raw
  bytes, direct sign docs, auth info, coins, public-key `Any`, and Celestia
  address helpers.
- Started the Tier-1 `cosmos-types` crate with Celestia address helpers, coins,
  protobuf `Any`, secp256k1 public-key packing, Cosmos SDK transaction domain
  types, raw transaction hashing, direct sign bytes, and focused API coverage.
- Added Go-generated `cosmos-types` conformance fixtures for Celestia
  addresses, coins, public keys, direct sign docs, raw transaction hashes, and
  malformed Cosmos SDK protobuf/domain cases.
- Added the Tier-1 `abci-types` crate spec for CometBFT ABCI++ request/response
  envelopes, vote extensions, validator updates, events, tx execution results,
  and snapshot protocol types.
- Started the Tier-1 `abci-types` crate with CometBFT v0.38 ABCI++ protobuf
  re-exports, request/response envelope wrappers, event/tx-result/snapshot
  domain types, validator updates, misbehavior descriptors, commit info, vote
  extensions, and focused API coverage.
- Added Go-generated `abci-types` conformance fixtures for ABCI++ request and
  response envelopes, vote extensions, events, tx execution results, snapshots,
  validator updates, and malformed protobuf/domain cases.
- Added the Tier-1 `nmt` crate spec for Celestia namespaced Merkle tree roots,
  namespace proofs, subroot path calculation, and Go conformance fixtures.
- Started the Tier-1 `nmt` crate with fixed namespace IDs, namespaced hashing,
  ordered tree roots, namespace inclusion/absence proof verification, subroot
  path calculation, and focused API coverage.
- Added Go-generated `nmt` conformance fixtures for primitive hashes, ordered
  tree roots, namespace/range proofs, subroot paths, and malformed domain
  cases.
- Added the Tier-1 `shares` crate spec for Celestia namespaces, fixed-size
  shares, compact transaction shares, sparse blob shares, padding, parsing, and
  Go conformance fixtures.
- Started the Tier-1 `shares` crate with 29-byte namespaces, info/reserved
  bytes, fixed-size share layout, compact and sparse splitting/parsing helpers,
  padding constructors, index-wrapper encoding, and focused API coverage.
