# ADR 0001 — Project direction (2026-10-14)

**Status:** Accepted. Supersedes the conflicting parts of
[`../superpowers/specs/2026-05-14-celestia-rs-design.md`](../superpowers/specs/2026-05-14-celestia-rs-design.md).

## Context

The top-level design was written in May 2026 against celestia-app v6. Upstream
has since shipped v7–v10 (v10.4.0 tagged 2026-10-01; it bumps celestia-core to
v0.42 and the celestia cosmos-sdk fork to v0.52.x, and adds `x/fibre`,
`x/valaddr`, `x/zkism`, `x/forwarding`). The owner re-stated the end goal and
answered a round of scoping questions; this ADR records the answers.

## End state

A Celestia **mainnet validator written in Rust** (state machine, consensus,
P2P, mempool, RPC), wire- and consensus-compatible with the Go
celestia-app/celestia-core validators, runnable on a **Mac Mini M4 (16 GB RAM,
512 GB SSD)**. Benchmarks against the Go implementation follow after mainnet
validation works.

## Decisions

| Topic | Decision |
|---|---|
| Consensus path | Rust app first, running behind Go celestia-core over ABCI++; then replace consensus, P2P and mempool with Rust. Go consensus is a stepping stone only. |
| Dependencies | Depend normally on common Rust crates, including protocol crates (`tendermint-proto`/`tendermint-rs`, `ibc-proto`/`ibc-rs`, `cosmos-sdk-proto`, `ics23`, …). Fork/vendor only when a crate diverges from Go behavior. **Celestia-specific logic lives in this repo**: NMT, rsmt2d, shares/square, blob, PrepareProposal/ProcessProposal, celestia modules. |
| History | Implement the **current mainnet app version** (and future upgrades). Bootstrap from state sync / trusted height. Genesis replay of v1…vN is a stretch goal, not required. |
| Go parity target | Whatever mainnet runs; bump the pinned Go versions on upgrades and port changes as discrete work items. |
| Strategy | **Early vertical slice.** Wire ABCI + app skeleton early, replay real blocks, let the first divergent app hash pick the next module/detail to port. |
| Differential oracle | A local Go `celestia-appd` devnet (built from module-proxy sources) produces blocks, app hashes and state exports; the Rust app must reproduce them byte-for-byte. Public mainnet RPC data is added when network policy allows. |
| Rollout | local mixed Rust+Go devnet → Arabica/Mocha full node → Mocha validator → mainnet full node → mainnet validator. |
| Signing | CometBFT-compatible local `priv_validator_key.json` / `priv_validator_state.json` with persistent double-sign guard. Remote signers are out of scope for now. |
| Storage | Validator-minimal pruning by default (512 GB SSD budget). No archival mode required. |
| Process | Go-generated fixtures and differential tests are **mandatory for consensus-critical code**. Short spec per milestone (no per-crate spec ceremony); coverage is a target, not a hard gate. |
| Workflow | Single agent; push directly to `main` after the local gate (`fmt`, `clippy -D warnings`, `test`). Progress lives in `MILESTONES.md` + commit log; blockers go to `OPEN_QUESTIONS.md` and work continues on unblocked items using Go behavior as the default answer. |

## Consequences

- The "vendor tendermint-rs / ibc-rs / lumina" guidance in the May design is
  replaced by normal dependencies (lumina is Celestia-specific, so its
  functionality stays implemented in-repo, as it already is).
- Fixture generation moves from v6.4.10 to the current release line; existing
  v6 fixtures stay valid where the behavior is unchanged and are regenerated
  where it is not.
- The milestone table is re-ordered around the vertical slice (see
  `MILESTONES.md`).
