# ADR 0008 — Mainnet recommendation scope

**Status:** Accepted (owner direction, 2026-10-24 America/Los_Angeles).
Supersedes ADR 0001's end state and rollout endpoint; its implementation
decisions remain in effect unless changed here.

## Goal

Earn enough evidence to recommend celestia-rs for Celestia mainnet's
non-archival RPC nodes, full consensus nodes and validators, alongside
the Go implementation. Running our own validator is an intermediate
canary, not the completion criterion. The initial compatibility baseline
is celestia-app **v10.5.0**, including the dependency versions that release
actually builds; v9.0.8 remains a historical upgrade/replay reference.
Follow future mainnet releases with explicit source-diff review and renewed
compatibility evidence.

## Scope

- Consensus and wire compatibility, application execution and state roots,
  transaction results, proposal/block/vote/evidence decisions, and
  interoperable sync with Go nodes.
- Non-archival RPC, REST and gRPC behavior used by clients and operators,
  including errors, proofs, subscriptions, indexing and retained history.
- Operational reliability: restart and crash recovery, pruning, snapshots,
  resource use under load, observability, and persistent double-sign protection.
- Linux and macOS deployment, including the existing Mac Mini M4 target.

Archival RPC service, HSMs and remote signers are later goals. Local-file
signing remains supported. An independent external security audit is
**optional**, not a prerequisite for general recommendation. This does not
remove F1.12's independent code-review requirement.

## Evidence and rollout

F1 remains the consensus-assurance gate. Its volume, coverage, replay and
elapsed-night criteria cannot be replaced by a successful live canary or
an assertion that the code is bug-free. Scope-wide recommendation also
requires RPC interoperability and operational qualification for the node
roles above.

Qualify an exact release-candidate commit. Record source commit, reference
versions and checksums, toolchains, target/platform, commands, seeds,
counts, outcomes and artifact locations. Missing evidence is pending,
not passing; a discovered mismatch is fixed and retained as a regression
before the affected campaign's clean count restarts. Evidence from older
commits is historical unless rerun on the candidate. Cross-platform and
nightly gates remain open until their results are available.

Expand adoption gradually after qualification. Publishing a release or
changing the owner's live validators is a separate operator action.
