# ADR 0009 — Three-night acceptance windows and PR auto-merge

**Status:** Accepted (owner direction, 2026-10-28 America/Los_Angeles).
Supersedes the observation windows and PR workflow in ADR 0001;
ADR 0008's recommendation scope remains in effect.

## Decision

The owner shortened the seven-to-fourteen-night acceptance gates to
**three consecutive clean nights** and requested that future celestia-rs
pull requests be ready for review with auto-merge enabled.

| Criterion | Observation window |
|---|---|
| C1.1 mainnet full node at the tip after the v10 upgrade | Three consecutive days, with matching app hashes and tx results |
| C1.2 execution differentials | Three consecutive clean nights |
| F1.2 no new consensus finding from any harness | Three consecutive clean nightly runs after the last fix |

Retain the other acceptance requirements: an exact candidate commit,
the finalized-transaction and per-operation input counts, scenario and
handler coverage, replay, cross-platform determinism, operations, and
independent review. A mismatch still requires a fix and a pinned regression
before the affected harness's clean count restarts. Shorter windows do not
make absent evidence passing evidence.

Open completed PRs as ready for review and enable GitHub auto-merge while
required CI runs. Fix failed checks and let GitHub merge after its required
checks pass. Do not bypass those checks or push directly to `main`.
This workflow does not authorize publishing releases or changing live
validators.
