# Reliability and maintainability evidence

The primary objective is a validator that agrees with Celestia's Go implementation,
preserves signing safety across failures, and keeps up with mainnet. No coverage
percentage, fuzz duration, or maintainability score proves the absence of bugs.
Stopping on an app-hash mismatch, corrupt signing state, or possible double signing
is a safety mechanism. Do not remove these stops to improve uptime.

## Measures worth tracking

| Measure | Decision and interpretation | Where it is measured |
|---|---|---|
| Go/Rust execution and app-hash mismatches | Hard failure for consensus paths. Retain input, reference version, seed, and both outcomes. Zero mismatches only describes the tested inputs. | `fuzz-exec`, nightly `fuzz-diff`, recorded conformance tests; `docs/fuzz-coverage.md` |
| Signing and recovery invariants | Hard failure. No signature before durable state; refuse conflicting votes/proposals after restart; refuse missing/corrupt state. Test failed writes as well as successful restarts. | `privval` tests, consensus WAL tests, mixed-validator and double-sign devnets |
| Uncovered lines, regions, and functions by crate/file | Worth tracking to locate gaps. Prefer tests of adversarial inputs and failure/recovery behavior. A workspace average can hide an untested signer; function/region counts are not branch coverage. | Nightly `coverage`: JSON, Markdown and annotated HTML artifacts |
| Critical-file uncovered-line budgets | Worth gating after measuring a reproducible baseline. Counts cannot be improved merely by adding covered boilerplate. Any budget increase needs a reason in the PR; decreases should accompany meaningful tests. | PR `coverage-critical` and nightly `coverage`; `scripts/quality/coverage-budgets.json` |
| Fuzz completion, executions, edges/features, throughput, peak RSS, corpus size | Worth tracking per target and build. Fail on crashes, timeout/OOM exits, or incomplete evidence. Use plateauing exploration to improve seeds and generators; do not optimize raw executions or corpus size as a product goal. | Ten nightly `cargo-fuzz` jobs: `metrics.json`, logs, toolchain, and corpora |
| Protocol/scenario coverage | More useful than the number of fuzz targets. Track successful/rejected messages, authority execution, app versions, upgrades and adversarial consensus sequences. Distinguish unreachable messages from harness gaps. | `docs/fuzz-coverage.md`, `crates/fuzz-diff/coverage-table.py`, `saturation-report.py` |
| Lint violations and dependency advisories | Existing strict Clippy and RustSec gates are useful maintainability/security controls. Fix causes; document justified exceptions. Do not replace well-tested dependencies merely to reduce a count. | PR `clippy` / `deny`, nightly `advisories`, `docs/security.md` |
| Height/signing lag, signed share, app-hash agreement, restart/recovery time and resource use | Closest operational evidence for validator reliability. Interpret against network conditions, catch-up, version and hardware. Track unplanned incidents separately from required safety stops. | Existing operator monitoring and soak reports; `packaging/macos/monitoring/README.md` |

Do **not** gate on a composite maintainability index, raw test count, lines of code,
or a universal function-length/complexity limit. These can reward splitting a
Go-compatible state transition into harder-to-audit pieces. Use complexity,
churn, duplication and dependency cycles as review prompts when a concrete change
is difficult to understand. A useful refactor reduces review effort while preserving
the conformance oracle; a lower score alone is insufficient justification.

## Coverage contract

The nightly test selection is `--workspace --all-features --locked`. The report
includes handwritten `src/` files in workspace tiers 0–5, excluding the two
generated protobuf crates (`celestia-proto`, `celestia-core-proto`), `src/gen/`,
separate `tests/`, `tests.rs`, `*_tests.rs`, and `testing.rs` files. Tooling,
integration harnesses and dependencies are outside the denominator, but their
tests still execute. LLVM includes inline test modules in other source files;
do not interpret these figures as an exact production-only percentage. The frame
codec's tests are separate so its budget covers implementation code only.

Reports retain covered and total counts, not just rounded percentages. A missing
crate is **not measured**, not 0% or 100%. The full nightly report fails if any
handwritten tier crate has no measured lines. Missing critical-file coverage,
test failures, and reporting failures also fail the job. Stable instrumentation
does not measure branches here; zero branch counters are not evidence of coverage.

`coverage-critical` runs only `abci-server` and `privval` tests on PRs. Its report
may also contain partially covered dependencies; compare those only with the same
suite. JSON records the suite, revision and dirty-worktree status, and artifacts
include `rustc -Vv`. Compare like toolchains, features, platform, exclusions and
test selection. A toolchain change may require a reviewed budget recalibration.

Initial budgets are **1 uncovered line in the frame codec** and **86 in the
signer**. These are regression ceilings from measurement, not acceptable long-term
targets. The new signer tests exercise failed atomic writes and fail-closed loading;
they reduce its uncovered lines from 94 to 86 (424 executable lines, Rust 1.98.0).
They do not simulate every storage failure or prove power-loss durability.

Reproduce the fast gate (Rust with `llvm-tools`, Python 3.11+):

```sh
cargo install cargo-llvm-cov --version 0.9.1 --locked
mkdir -p target/quality
cargo llvm-cov --locked -p abci-server -p privval --json --output-path target/quality/llvm.json
python3 scripts/quality/coverage.py target/quality/llvm.json \
  --policy scripts/quality/coverage-budgets.json --out target/quality \
  --suite 'cargo llvm-cov --locked -p abci-server -p privval'
```

For the full report, use `cargo llvm-cov --workspace --all-features --locked`,
keeping the JSON output arguments, and pass the matching `--suite` plus
`--require-all` to the reporter. `cargo llvm-cov report --html --output-dir
target/quality/html` produces the annotated source without rerunning tests.

## Fuzz evidence and cost

Each target keeps the existing 600-second fuzzing budget and 60-minute job limit
(which includes compilation), with a 2 GiB RSS limit and a 30-second per-input
timeout. These are per-target limits: ten targets consume up to 100 minutes of
fuzz execution per night, plus compilation. The workspace coverage job is capped
at 90 minutes; the additional PR coverage job at 20 minutes. There is no paid
coverage service or new live-mainnet CI dependency.

Checked-in seeds are always supplied. Newly discovered inputs go to a separate
cached corpus; successful runs save it under a new immutable cache key. Cache
eviction can reset discovery, so the committed regression seeds remain the minimum
reproducible corpus. All runs upload logs, metrics, corpus, and available crash
artifacts for 30 days; failures do not overwrite the successful corpus cache.
The compiler and cargo-fuzz versions accompany each report. A missing final
libFuzzer summary cannot become a successful zero-findings run.

The fuzz release profile explicitly enables overflow checks, matching the shipped
node, including direct release builds. Cargo-fuzz also enables these by default.
Existing targets still use `--sanitizer none`: they exercise Rust panic,
resource and harness assertions, **not AddressSanitizer memory-safety coverage**.
The ABCI target also checks accepted lengths against prost's independent varint
decoder; a crash-only harness would miss silently truncated length prefixes.

Edge and feature counts are comparable only for the same target/instrumentation;
they are not percentages, cannot be added across targets, and do not measure how
much of the entire consensus state machine was fuzzed. Stateful Go differentials,
recorded mainnet replay, cross-architecture determinism, and chaos/restart tests
remain necessary alongside these byte-oriented targets.

## Reviewing progress

GitHub job summaries give the current results; download `workspace-coverage`,
`critical-coverage` or `cargo-fuzz-<target>` artifacts to compare revisions. JSON
uses `schema_version: 1`. Thirty-day retention supports short-term comparisons;
archive artifacts alongside a release for longer history. This PR does not create
a permanent metrics database or claim a live-mainnet reliability baseline.

Prioritize uncovered consensus, signing, persistence and untrusted-input paths.
Pair each incident/fuzz finding with a minimized regression and, where possible,
a Go compatibility assertion. Expand critical budgets after establishing stable
measurements; never raise coverage by suppressing errors or excluding difficult
production code. Track flakiness as failed/rerun outcomes rather than rerunning
until green. Passing checks provide evidence for review, not a release certificate.
