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
