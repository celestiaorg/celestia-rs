# Differential branch profile, 2026-11-03

A bounded full profile on **`5a22054579a1a152cf36028b9e6dd0ba8e11908c`**
completed all 179 selected cases with zero differential mismatches. The exact
candidate wrapper recorded a clean identical tree before and after the run
(`062f4f0fef91e28e7933beeca51fc460e394d8fd`), unchanged Go binary hashes and
exit status 0. This is one completed check, not mainnet recommendation or
completion of F1.2/F1.3. Evidence does not transfer automatically to later commits.

[The compact receipt](evidence/fuzz-profile-5a22054.json) retains toolchains,
source identity, Go binary hashes, per-crate branch counters and hashes of the
raw reports. Full artifacts remain in the engineering evidence workspace at
`/workspace/scratch/celestia-week/full-branch-5a22054/`; wrapper evidence is in
`qualification-branch-5a22054/`. These are local artifacts, not a public artifact
archive. The receipt alone cannot reproduce the retained raw observations.

## Selection and volume

The selection was the committed `coverage.sh` at this candidate: generated
block delivery with wire mutations, gas sweeps and recheck; governance-pass
runs with simulation and required legacy-deposit success; four simulated
execution modes; proposal mutations; four generated upgrade seeds; and the
recorded upgrade. Go v9.0.8 and v10.5.0 were both built with Go 1.26.9. A
read-only mainnet check before launch found providers reporting v10.5.0 and
v10.4.0 at the same height, 15,556,636. The oracle targets remain the documented
v10.5.0 and historical v9.0.8; provider heterogeneity was not treated as agreement
on the deployed patch version.

| Scope | Runs | Generated blocks | Finalized transactions | Go code 0 |
|---|---:|---:|---:|---:|
| Complete requested delivery budgets | 56 | 8,400 | 78,185 | 34,606 |
| Recorded upgrade, intentional early halt | 1 | 9 of 150 | 43 | 32 |
| Diagnostic delivery totals | 57 | 8,409 | 78,228 | 34,638 |

All 179 case exit codes are zero, but the recorded upgrade halts after its
recorded boundary. Its 43 transactions are excluded from the complete-budget
subtotal. This profile is not the million-transaction saturation campaign,
50 upgrade seeds, per-operation input gate or three scheduled clean nights.

## LLVM branches and message gaps

The run used nightly-2026-11-02 (rustc commit
`3e92e1a2d20b991ef23a6c6064c34d57ee8c98d3`, LLVM 23.1.5),
cargo-llvm-cov 0.9.1 and `COVERAGE_BRANCH=1`. LLVM's handwritten-tier summary
reports **5,321 covered of 13,450 branch arms**, with **8,129 uncovered**.
This includes linked node and networking code that an ABCI application harness
does not drive. It is neither production-only nor semantic-path coverage.

| Selected crate | Covered arms | LLVM summary arms |
|---|---:|---:|
| app | 434 | 890 |
| x/authz | 93 | 174 |
| x/packetforward | 71 | 136 |
| x/staking | 309 | 462 |
| x/zkism | 104 | 172 |

`branches.json` retains each exported raw region and its uncovered arms with
review pending. Its raw-region denominator differs from LLVM's summary;
instantiations are not deduplicated or substituted for summary counts. Reasons
for individual uncovered arms remain unfinished.

The regenerated message table has 267 entries and 23 message types without
observed Go success, listed in the receipt. This includes nested data types and
messages that may be disabled by the reference application. The list does not
establish that all 23 are reachable harness gaps. Classifying them requires
reference-source and execution evidence, rather than forcing success or dropping
them from the report. In both pinned SDK 0.52.8 and 0.52.12,
`x/auth/vesting/msg_server.go:112–117` unconditionally rejects periodic and
permanent-locked vesting creation with `ErrInvalidRequest`; absence of success
for these two messages is expected reference behavior. That source observation
does not classify the other entries or establish complete rejection coverage.

This candidate predates the SDK-indexed handler-error instrumentation. Its
`CODE` counts identify failed transactions by their first message and cannot
establish per-handler error coverage. The message table must not be retroactively
marked as measuring the newer counters. Nested proposal submission is also not
proof of execution; actual passed-proposal executions are reported separately.

## Retained failures and remaining gates

The preceding clean-source profile on `47bd1a2` failed 20 required legacy-deposit
coverage checks despite zero differential mismatches. Its logs and profiles
remain separately retained under `full-branch-47bd1a2/`; it has not been relabeled
passing. PR #253 corrected deposit generation to query the proposal's applicable
minimum, and this fresh candidate completed those checks.

Exact-candidate volume and scenario coverage, individual uncovered-arm reasons,
per-handler errors and panic evidence, clean scheduled nights, mainnet-shaped
upgrade/replay evidence, cross-platform determinism, independent review and the
three-day mainnet soak remain governed by MILESTONES.md and ADRs 0008/0009.
No live node, validator, key or stake was changed by this profile.
