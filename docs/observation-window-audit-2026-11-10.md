# Exact-candidate observation audit — 2026-11-10

The retained evidence does **not establish all acceptance criteria** for
[#298](https://github.com/celestiaorg/celestia-rs/issues/298) or
[#299](https://github.com/celestiaorg/celestia-rs/issues/299). Neither issue
is closed. The requested interval is **2026-11-06T05:43:12Z through
2026-11-09T05:43:12Z**, exactly 72 hours. Elapsed time alone is insufficient.

Collection was read-only with respect to services. No updater, installer,
validator binary, signing key, keyring, SMTP credential or secret environment
file was executed or read. Existing collectors continued independently.
The offline auditor from merged PR #346, commit
`8ffc98a50fb9bf6e4ec8bc29351617f7cd6c346c`, ran in a separate clone under
`/private/tmp/celestia-window-audit/checkout`, without touching deployed source
or services. Its ten coverage tests passed.

## Candidate and executable provenance

Candidate: `b459a6e041b2675582019cde5c75ecf2969318ba`.
The [Nov 5 investigation](live-validator-evidence-2026-11-05.md) recorded both
loaded executable inodes matching installed files and binary SHA-256
`fb66fba07c468595cc2801c9355054d8518149f4a62bb0ce78caac4d1d76efde`.
The retained Nov 6 start record ties mainnet PID 74147 to loaded/installed
inode 24168243 and that embedded commit after the controlled configuration
restart. Mocha's recorded post-restart PID was 72787. The earlier record and
later periodic receipts support this identity; the original binary is no
longer installed, so its digest was not independently remeasured in this audit.
No retained continuous binary-byte measurement exists.

839 periodic receipts per network identify that full candidate during the
window. The observed installed collector identifies the loaded executable by
`ps`, matching `lsof`/`stat` inode, then the explicit binary's `version --long`.
These receipts assert the observed commit; they do not retain the raw inode
probe or binary hash at each check. The start note and periodic checks are
stronger provenance than the per-height collectors, which **hard-code** the
candidate string. Their receipts keep that string after deployments and must
not independently authenticate the process identity.

Node logs contain no start/stop within the requested interval. Deployment logs
retain the hold through 2026-11-09T06:14:21Z. The first subsequent changes are
Mocha `b459a6e -> 3a6c71c` at 07:17:40Z (height 2,454,936), and mainnet at
07:34:38Z (height 15,727,376). Mainnet next starts at 07:35:08.572305Z;
these are outside the requested window and end any extension of this candidate
run. Further changes to `69df8af`, `a9f396b` and `8ffc98a` are later deployments,
not evidence that the earlier 72 hours used those candidates.

There is a baseline conflict: the start note says mainnet height 15,634,043
at baseline. Timestamped execution logs show height 15,634,037 committed at
05:43:11.966880Z and 15,634,038 at 05:43:14.816968Z. This audit retains the
conflict and bounds execution coverage using the latter logs; it does not
silently substitute the note's height.

## Exact height and receipt coverage

All ranges below are inclusive. Execution timestamps define the required
range; receipt timestamps are comparison times, not block times.

| Chain | Logged execution in the interval | First / last execution UTC |
|---|---|---|
| `celestia` | 15,634,038–15,725,032 (90,995 heights) | Nov 6 05:43:14.816968 / Nov 9 05:43:10.067260 |
| `mocha-5` | 2,362,383–2,452,952 (90,570 heights) | Nov 6 05:43:12.409526 / Nov 9 05:43:10.807857 |

Both execution logs have every height in these ranges, with no internal
height gaps. A log line proves recorded execution, not independent at-tip lag.

Mainnet per-height reference: `https://rpc.celestia.pops.one`; local RPC:
`http://127.0.0.1:26757`. Per-height app-hash receipts omit the endpoint;
the retained collector script establishes it. Periodic mainnet reference:
`https://celestia-rpc.publicnode.com`. Mocha periodic reference:
`https://rpc.celestia-mocha.com`; local RPC: `http://127.0.0.1:26657`.
Successful receipt chain IDs agree (`celestia` or `mocha-5` respectively).
Failed status requests leave reference chain identity unknown, not conflicting.

| Mainnet receipt evidence collected within the window | App hash | Transaction results |
|---|---|---|
| Rows / successful rows | 95,675 / 95,402 | 95,679 / 95,064 |
| Unavailable rows / distinct affected heights | 273 / 154 | 615 / 273 |
| Heights with at least one success | 90,978 | 90,982 |
| Successful range before accounting for unavailable attempts | 15,634,044–15,725,021 | 15,634,044–15,725,025 |
| Heights with success and no unavailable attempt | 90,824 | 90,709 |
| Never observed in these receipts | 15,634,038–15,634,043; 15,725,022–15,725,032 | 15,634,038–15,634,043; 15,725,026–15,725,032 |
| Concrete mismatch / contradictory successful payload | 0 / 0 | 0 / 0 |

The app-hash collector compares the two committed headers at H+1. Separately,
this audit joins the actual logged execution `app_hash` at H to each retained
reference hash: **90,978 heights match**, no mismatch or conflicting reference
hash, with the same uncovered leading/trailing heights. This cross-check avoids
mistaking agreement between two headers for direct execution-state evidence.

Transaction comparisons normalize ordered results and compare **`code`,
`data`, `gas_wanted`, `gas_used`**, counts and SHA-256 of normalized JSON.
There are 391,651 transactions at heights with successful comparisons. Logs,
events and other ABCI fields were not compared. There are 344 unavailable
reference-status rows and 271 unavailable reference-block-results rows.
App-hash failures have blank reference chain/hash without an error reason;
the script cannot establish their underlying transport/server cause.

Every unavailable attempt, its UTC time and height is retained in
[apphash-failures.json](evidence/observation-window-2026-11-06/apphash-failures.json)
and [tx-results-failures.json](evidence/observation-window-2026-11-06/tx-results-failures.json).
Every covered, missing and unavailable height range is enumerated in the
corresponding [app-hash](evidence/observation-window-2026-11-06/apphash-audit.json)
and [transaction](evidence/observation-window-2026-11-06/tx-results-audit.json)
audits. In particular, `uncontested_missing_ranges` and `unavailable_ranges`
include identity-unavailable failures: PR #346's original `rejected_ranges`
alone does not include failures rejected earlier by its identity check.
The original auditor's counts and ranges are preserved alongside these
additional conservative summaries. Both original audits report incomplete.

Later comparisons before candidate replacement successfully reach the required
last height for both kinds, but still never cover 15,634,038–15,634,043.
[later-comparisons.json](evidence/observation-window-2026-11-06/later-comparisons.json)
keeps those retrospective comparisons separate. They do not erase earlier
unavailable attempts or establish observation at that earlier time.

The periodic collector additionally recorded 839/839 mainnet app-hash matches
and 835/839 Mocha matches. Its first mainnet height 15,634,041 fills only one
leading app-hash height above. Thus **all collected app-hash sources still
lack mainnet heights 15,634,038–15,634,040 and 15,634,042–15,634,043**.
Mocha unavailable checks: 2,397,033 (Nov 7 09:15:01Z), 2,402,841
(13:52:46Z), 2,408,358 (18:15:57Z), 2,441,398 (Nov 8 20:32:14Z).
Other Mocha heights outside the explicitly enumerated periodic success ranges
have no app-hash receipt in this collection. No every-height Mocha result
collection exists; the retained five-height canary 2,362,294–2,362,298 is
before this window and establishes no transaction-result coverage inside it.

## Operation, signing and monitoring

Prometheus was queried at the fixed endpoint time with raw `[72h]` matrix
selectors, not interpolated query-range values. See
[summary.json](evidence/observation-window-2026-11-06/summary.json).
Each node target has 17,280 successful scrapes, zero failed scrapes, and no
internal sample spacing over 20 seconds. Collector series have 17,279 samples
from Nov 6 05:43:26.636Z through Nov 9 05:42:58.254Z, no internal spacing over
20 seconds, maximum collector timestamp age 32.680 seconds. Different counts
reflect their own scrape timestamps; they are not evidence of one failed scrape.
Endpoints and time between observations remain unobserved.

Both networks have observed RPC/process-up=1, catching-up=0, signing-info-found=1,
jailed=0 and tombstoned=0 throughout available samples. Maximum observed block
age is 40.211s mainnet / 42.650s Mocha. These support sampled near-tip operation.
**No continuous independent reference-height series was retained.** Collectors
fetch reference height to limit their cursor, but discard it; parity at an old
height does not prove that execution was at reference tip. The conflicting
baseline note cannot repair this missing history.

Mainnet local cumulative missed-block count grows 2→259 (257 observed misses),
Mocha 1→212 (211), with no counter resets. Last-signed-height series advance
15,634,037→15,725,028 and 2,362,387→2,452,952. These establish ongoing local
signing observations, not a per-height independently verified signature ledger.
Rolling on-chain missed-window gauges are separate and their decreases are
normal window movement, not process resets. The baseline note records 20/20
recent independent commits on each network; its sample precedes qualification.
A full-window independent signed/missed/nil/error ledger is not retained.
Mainnet `ValidatorMissingBlocks` fired in two sampled spans on Nov 7
07:28:13.659–07:31:43.659Z and 07:34:43.659–07:34:58.659Z; alert evaluation is not delivery evidence.

No guard refusal, panic, start or stop matched the window's inspected node logs.
The mainnet refusal counter stays 0; Mocha's cumulative log-derived refusal
counter stays 3, with no new increment. This supports clean observed Mocha
behavior on the candidate, while retaining the historical refusals.

## Codesigning and historical guard refusal

The [retained codesigning investigation](codesigning-incident-2026-11-05.md)
establishes a deliberate updater deployment at Nov 5 23:12:17Z, followed at
23:12:17.433Z by an AMFI launch-constraint rejection (no CMS blob / unrecoverable
CT signature issue), `xpcproxy` termination with `OS_REASON_CODESIGNING`, then
successful launch at 23:12:47.551Z and a healthy signing check at 23:14:12Z.
This is a failed launch during deployment, not an established Rust panic.
The deeper reason the immediate launch failed and the retry succeeded remains
unknown; signing-cache or file-replacement races are hypotheses. The narrow
unified-log query in this audit returns zero retained events, so the dated
investigation, rather than a newly recovered raw AMFI record, supports that
sequence. A stale last-exit reason is not a new incident in the target window.

Raw node/updater logs retain Mocha's Oct 31 deployment at 04:11:17Z, stop at
04:11:17.647178Z, start at 04:11:18.293690Z, consensus-message replay at height
2,179,423 at 04:11:20.071157Z, and the round-0 `conflicting data` signing refusal
at 04:11:22.088532Z. The guard refused a conflicting signing request; this
alone is not evidence of a double-sign. The documented restart/WAL scenario
and regression in PR #228 explain the known refusal mechanism. Merged fix
`880eb5d` is in the candidate's ancestry and its already-voted guard is present
in candidate source. These retained logs support the restart association;
without inspecting signing state/WAL they do not reconstruct every vote of
this particular incident. No such secret/state inspection was performed.

## Retention, reproducibility and remaining criteria

Raw artifacts remain privately under `/private/tmp/celestia-window-audit/raw`:
receipt snapshots, node/deployment/watchdog logs, non-secret baseline records,
collector source and raw Prometheus responses. These are copied byte snapshots
of live files, not atomic filesystem snapshots or authenticated receipts.
The deterministic compressed [manifest](evidence/observation-window-2026-11-06/raw-manifest.json.gz)
contains each source-relative filename, byte count and SHA-256. It contains no
raw node-log contents, private service paths or credentials. Derived receipt
inputs are local `apphash-window.jsonl` and `tx-results-window.jsonl`, assembled
in lexicographic source filename order, preserving raw lines whose UTC check
time lies in the requested window. Audit command for each kind:

```sh
python3 scripts/quality/receipt_coverage.py --chain celestia \
  --candidate b459a6e041b2675582019cde5c75ecf2969318ba \
  --first 15634038 --last 15725032 --kind apphash apphash-window.jsonl
# Repeat with --kind tx-results tx-results-window.jsonl. Both exit 1.
```

[digests.json](evidence/observation-window-2026-11-06/digests.json) records SHA-256
for both derived inputs and all sanitized evidence artifacts. Summaries and
failure details are published; original receipt-file hashes are in the manifest.
Full private raw data is required to independently rerun the entire audit.

Exact unmet criteria: leading mainnet per-height parity/results coverage;
unavailable checks retained as unknown despite retries; full-window independent
reference-height lag evidence; complete independent signing coverage; and
stronger retained interval binary-hash provenance than the dated measurement
and periodic commit assertions. Mocha provides clean sampled behavior, but only
periodic app hashes and no in-window transaction-result receipt ledger.
Monitoring and execution logs substantially improve the operation record;
they cannot fill missing independent observations. A new qualifying interval
would need separate authorization and evidence collection; this audit changes
no services and does not arrange or start one.
