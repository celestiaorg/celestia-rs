# Changelog

## Unreleased

### Added

- Added the M3 transaction-and-ante milestone index plus the first `ante`
  crate spec, defining the transaction validation, signature, gas, fee, and
  account-state trait boundary for the next implementation section.
- Started the Tier-3 `ante` crate with the public ante skeleton, gas-meter
  behavior, stateless `TxRaw` validation, and focused API tests.
- Expanded `ante` API coverage for signed deliver-mode execution, sequence
  increments, public-key persistence, sequence mismatches, and invalid
  signatures.
- Added `ante` fee-path coverage for explicit fee payers, minimum gas-price
  checks, insufficient fees, and insufficient funds without sequence mutation.
- Added the first Go-generated `ante` conformance fixture for stateless
  transaction validation and registered it with `xtask refresh-fixtures`.
- Added `cargo xtask repo-health` to codify the local fmt, clippy, test,
  fixturegen, and fixture-inventory gates before moving between milestone
  sections.
- Added a DB-backed `fixturegen replay-state` utility plus the M2 replay-state
  fixture hardening spec for replacing the synthetic M2 acceptance window with
  real mainnet state diffs.
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
- Added Go-generated `shares` conformance fixtures for namespaces,
  info/reserved bytes, builders, compact transactions, sparse blobs, padding,
  and malformed share cases.
- Added the Tier-1 `square` crate spec for original data square layout,
  wrapped PFB placement, blob commitment rules, erasure extension, DAH roots,
  and Go conformance fixtures.
- Started the Tier-1 `square` crate with blob transaction wrapper encoding,
  inclusion-rule math, square sizing, `write_square` layout, and focused API
  coverage.
- Expanded the Tier-1 `square` crate with builder/export mechanics, wrapped
  PFB index assignment, transaction/blob share ranges, and square
  deconstruction helpers.
- Added `square` blob share commitment helpers with 29-byte namespace NMT
  roots and Merkle mountain range sizing.
- Added the `square` data availability header value type with protobuf
  conversion, root-count validation, and DAH hash calculation.
- Added `square` Leopard erasure coding, extended data-square import/export,
  row/column root calculation, and DAH construction from EDS roots.
- Added initial Go-generated `square` conformance fixtures for power-of-two
  sizing helpers and blob inclusion-rule placement.
- Added Go-generated `square` conformance fixtures for blob transaction
  wrappers and blob share commitments.
- Added Go-generated `square` conformance fixtures for builder/export behavior,
  ordered construction errors, share ranges, and wrapped PFB indexes.
- Added Go-generated `square` conformance fixtures for direct square layout,
  empty squares, wrapped PFB extraction, and malformed square sizes.
- Added Go-generated `square` conformance fixtures for transaction and blob
  share ranges, including non-PFB and out-of-range errors.
- Added Go-generated `square` conformance fixtures for square deconstruction,
  blob transaction reconstruction, and PFB decoder error handling.
- Added Go-generated `square` conformance fixtures for Leopard erasure
  extension, DAH roots/hashes/protobuf, and malformed square inputs.
- Added signer-aware share version 1 support for `shares` sparse blobs and
  `square` blob transaction wrappers.
- Added a cached mainnet block DAH fixture for height 10126899 and the
  `m1-integration` acceptance scaffold.
- Made the `square-mainnet-blocks` fixture generator consume every embedded
  `mainnet_block_*.json` testdata file and aligned fixturegen CI with Go
  1.24.11.
- Added a `square-fetch-mainnet-blocks` fixturegen utility to cache CometBFT
  RPC block responses for future mainnet corpus expansion.
- Expanded `square-mainnet-blocks` to 100 cached mainnet app-v6 blocks and
  marked M1 complete.
- Added the M2 state-and-storage spec index and the first M2 `iavl` crate spec.
- Started the Tier-2 `iavl` crate with an in-memory mutation API, version
  commits, deterministic root hashing, range iterators, and placeholder proof
  material.
- Added Go-generated `iavl` conformance fixtures for roots, iterators,
  ICS-23 proof bytes, and malformed API cases, then aligned Rust IAVL root
  hashing and versioned AVL tree shape with Go for committed root/iterator
  fixtures.
- Replaced placeholder `iavl` proof operations with fixture-backed ICS-23
  membership and non-membership proof bytes plus root verification.
- Expanded `iavl` malformed coverage for invalid proof verification inputs,
  proof operation payload mismatches, and impossible private node shapes.
- Added the M2 `kvstore` crate spec for IAVL-backed KV access, cache branches,
  prefix stores, gas accounting, and conformance fixtures.
- Started the Tier-2 `kvstore` crate with an IAVL-backed store, write-back
  cache, prefix-scoped view, gas-metered access, SDK byte helpers, and focused
  API coverage above the M2 threshold.
- Added Go-generated `kvstore` conformance fixtures for basic IAVL-backed
  operations, cache write-back, prefix stores, gas accounting, and malformed
  API cases.
- Added the M2 `ics23-proofs` crate spec for SDK proof-op wrapping,
  membership / non-membership verification, simple commitment proofs, and
  conformance fixtures.
- Started the Tier-2 `ics23-proofs` crate with SDK proof-op wrappers, IAVL
  membership / non-membership verification, batch and compressed proof root
  calculation, CometBFT `ProofOp` protobuf compatibility, and focused coverage
  above the M2 threshold.
- Added Go-generated `ics23-proofs` conformance fixtures for IAVL commitment
  ops, simple Merkle store-root proofs, CometBFT `ProofOp` / `ProofOps`
  encoding, and malformed proof-op cases.
- Added the M2 `multistore` crate spec for Cosmos SDK commit info protobufs,
  deterministic multi-store app hashes, store-root proof ops, and cache
  multi-store behavior.
- Started the Tier-2 `multistore` crate with in-memory store mounting,
  commit-info protobuf encoding, deterministic app-hash composition,
  store-root proof ops, commit history, and clone-backed cache multi-store
  write-back.
- Added Go-generated `multistore` conformance fixtures for app-hash commits,
  cache multi-store write-back, store-root plus IAVL proof-op chaining, and
  malformed multi-store cases.
- Added the `m2-integration` acceptance scaffold with a Go-generated
  mainnet-height state-diff fixture that verifies Rust multi-store app hashes
  against Go SDK roots.
- Trimmed redundant derived ODS share dumps from the 100-block
  `square-mainnet-blocks` fixture so the generated corpus stays below
  repository single-file warning thresholds while preserving DAH root coverage.
