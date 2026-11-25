# Validator track record — November 25, 2026

This report counts the public on-chain record of the mainnet canary and the
Mocha validator from creation to a fixed height. Every number comes from
public RPC/REST data or from audits already retained in this repository.
It also rechecks the 45 mainnet heights that the
[November 20 audit](validator-observation-audit-2026-11-20.md) excluded.
No live service, key or configuration was touched. Item 10 in
[OPEN_QUESTIONS.md](../OPEN_QUESTIONS.md) stays open.

## The 45 excluded mainnet heights

**All 45 heights were reference-data gaps. None was a disagreement.**

The November 20 mainnet collector retained 50 failed height attempts in
[failed-heights.jsonl](evidence/validator-observation-audit-2026-11-20/mainnet/failed-heights.jsonl).
49 fall inside the required range 15,815,626–15,906,676, at 45 distinct
heights (41 heights with one failed attempt, 4 with two). The 50th,
height 15,906,683, is outside the range. Every one of the 50 attempts has
outcome `unavailable` and the error `HTTP Error 429: Too Many Requests`
from the reference relay. The 49 in-range attempts break down by the
request that failed:

| Failed reference request | Attempts |
|---|---:|
| `/commit?height=H` | 23 |
| `/block_results?height=H` | 16 |
| `/block?height=H+1` | 9 |
| `/commit` and `/block_results` together | 1 |
| **Total** | **49** |

There are no HTTP 502/503 responses, timeouts, parse errors, reference
catch-up or value mismatches among the height attempts. (The 31 failing
*status samples* in the same audit are a separate record.) The collector
stops on any `mismatch` outcome, and it advances its next height only after
a `match`. Its only terminal event is the normal deadline at November 15
05:29:01 UTC with next height 15,906,759, so each of the 45 heights later
produced a `match` row on the Mac. Those rows are in the machine-local
`heights.jsonl`, not in git.

### Independent recheck

On 2026-11-25 at 01:59:44 UTC, `GET /commit?height=H+1` was fetched for each
of the 45 heights from three public providers: `https://rpc.celestia.pops.one`
(the provider behind the original relay), `https://public-celestia-rpc.numia.xyz`
and `https://celestia.rpc.kjnodes.com`. Each response was checked for chain ID
`celestia` and height H+1.

| Check, per height | Heights passing |
|---|---:|
| All three providers return identical `app_hash`, `last_results_hash`, `last_block_id.hash` and block hash | 45 / 45 |
| Reference `app_hash` at H+1 equals the local `app_hash` logged for H | 45 / 45 |
| Reference `last_block_id.hash` at H+1 equals the local block hash logged for H | 45 / 45 |

The local values come from the node's own `committed block` log line, which
the collector copied into each failed row. Results are in
[recheck-45-heights.json](evidence/validator-track-record-2026-11-25/recheck-45-heights.json).

**What the retained evidence does not hold:** the local transaction-result
commitment for these 45 heights. The failed rows store no local results, and
the later matching rows are machine-local. The recheck records the reference
`last_results_hash` from three agreeing providers. It cannot compare that hash
to a local value from git. The result comparison for these heights rests on
the collector's match-or-stop rule, not on retained bytes.

## Identities

| Field | Mainnet `celestia` | Mocha `mocha-5` |
|---|---|---|
| Operator | `celestiavaloper16v6qxgj2w996jt49tngf9fkvxw6j7u0uh6d9ec` | `celestiavaloper13v73gcz3k923ajt23alr5h3wuvsgqxu3jag0sd` |
| Account (same bytes) | `celestia16v6qxgj2w996jt49tngf9fkvxw6j7u0uj90u07` | `celestia13v73gcz3k923ajt23alr5h3wuvsgqxu3hz2kxt` |
| Consensus address | `7FC8073F6F0DE936593BBBF6E6EBE37E17487615` | `64910794C3B7404929AB3CF18063A6B8CD3032D7` |
| Consensus Bech32 | `celestiavalcons10lyqw0m0ph5nvkfmh0mwd6lr0ct5sas4ugtync` | `celestiavalcons1vjgs09xrkaqyj2dt8nccqcaxhrxnqvkhqwen2s` |

Identities are from the [November 5 evidence](live-validator-evidence-2026-11-05.md).
The mainnet consensus address is the one the November 20 collector checked.

## Track record

**As of:** mainnet height **16,206,000** (2026-11-25 01:54:13 UTC); Mocha
height **2,931,000** (2026-11-25 02:01:44 UTC). All ranges are inclusive.

| Metric | Mainnet | Mocha |
|---|---|---|
| `MsgCreateValidator` | 15,104,013, Oct 19 17:38:28 | 1,720,896, Oct 15 23:19:26 |
| First active-set interval | 15,104,015–15,125,849 (17.7 h) | 1,720,898–1,856,334 (4.49 d) |
| Jailed (`missing_signature`) | 15,125,848, Oct 20 11:18:41 | 1,856,333, Oct 20 11:05:44 |
| Out of the active set | 15,125,850–15,129,614 (3,765 heights, 3.0 h) | 1,856,335–1,860,317 (3,983 heights, 3.2 h) |
| `MsgUnjail` | 15,129,613, Oct 20 14:18:26 | 1,860,316, Oct 20 14:15:33 |
| Second active-set interval | 15,129,615–as of (35.48 d) | 1,860,318–as of (35.49 d) |
| Days in the active set | **36.22** | **39.98** |
| Status / jailed at as-of height | bonded / false | bonded / false |
| Tombstoned | false | false |
| Slash events, all heights | 1 (the downtime jail; 0 burned) | 1 (the downtime jail; 0 burned) |
| Double-sign slash events | 0 | 0 |

### Signing

Counts use the chain's own slashing record. The `liveness` event at block H
reports that the validator's vote for H−1 was not a commit signature (absent
or nil). Signed = counted heights − missed.

| Range of votes | Mainnet signed / missed | Uptime | Mocha signed / missed | Uptime |
|---|---:|---:|---:|---:|
| First interval (mainnet 15,104,015–15,125,847; Mocha 1,720,898–1,856,332) | 11,819 / 10,014 | 54.134% | 124,991 / 10,444 | 92.289% |
| — of which the miss run before jailing | 0 / 9,991 | | 0 / 9,991 | |
| Second interval (mainnet 15,129,615–16,205,999; Mocha 1,860,318–2,930,999) | **1,072,737 / 3,648** | **99.661%** | **1,066,888 / 3,794** | **99.646%** |
| Both intervals | 1,084,556 / 13,662 | 98.756% | 1,191,879 / 14,238 | 98.820% |
| Current slashing window (10,000 counted heights) | — / 18 | | — / 11 | |

The miss run before jailing covers mainnet votes 15,115,857–15,125,847
(block times Oct 20 03:07:11–11:18:39 UTC) and Mocha votes
1,846,342–1,856,332 (about 03:07–11:05 UTC). Each run is 9,991 heights,
the count in the jailing `liveness` event. Mainnet's last signed vote before
it was 15,115,856. It started at the same time on both
networks, about 38 minutes before mainnet's v10 upgrade height 15,116,658.
This matches the Mac network outage recorded under M14 in
[MILESTONES.md](../MILESTONES.md).

Cross-checks:

- The signing-info `index_offset` at the as-of height is 1,076,385 on
  mainnet and 1,070,682 on Mocha. Each equals the second interval's counted
  heights exactly. A jail resets this offset, so there was no jail and no gap
  in the active set during the second interval.
- Mainnet votes 15,815,626–15,906,676 (the November 12–15 window) have 209
  `liveness` events, equal to the 209 missed in the
  [November 20 audit](validator-observation-audit-2026-11-20.md). Mocha's
  2,543,075–2,633,657 has 147, again equal.
- November 6–9 window: mainnet 15,634,038–15,725,032 has 257 events and
  Mocha 2,362,383–2,452,952 has 211. Both equal the local missed-counter
  increments in the [November 10 audit](observation-window-audit-2026-11-10.md).
- Direct `/commit` reads of mainnet 16,205,001–16,206,000 find 998 signed and
  2 missing (16,205,476 and 16,205,654). Mocha 2,930,701–2,931,000 finds 299
  signed and 1 missing (2,930,980). The `liveness` events at the next heights
  are exactly these.
- Two providers per network return the same event counts for every range.
  One query differs: Numia's mainnet `slash` search with no height filter
  returned 0. The same search with height bounds returned 1, as did Pops.

The first-interval counts assume the validator stayed in the active set
between the checked endpoints. No unbonding or other slash event occurs there,
and the `liveness` events continue to the jail height.

### Proposals

| Range | Mainnet proposed | Mocha proposed |
|---|---:|---:|
| Mainnet 16,205,001–16,206,000; Mocha 2,930,701–2,931,000 (this report) | 0 of 1,000 | 1 of 300 (2,930,722) |
| Mainnet 15,626,108–15,626,207; Mocha 2,354,476–2,354,575 ([Nov 5](live-validator-evidence-2026-11-05.md)) | 0 of 100 | 0 of 100 |
| Full history | **not established** | **not established** |

No public index records proposers, so a full count needs every header
(about 55,000 `/blockchain` pages for mainnet). That was not done. For
context only, mainnet voting power at the as-of height is 61,303,623 of
465,117,017,699,641 bonded utia (1.3 × 10⁻⁷).

### App-hash and result comparisons (mainnet)

These come from audits already in the repository. Ranges do not overlap.

| Source | Heights | App hash matched | Tx results matched | Mismatches |
|---|---|---:|---:|---:|
| Periodic checker, Oct 19 14:35–Nov 5 23:19 UTC ([Nov 5](live-validator-evidence-2026-11-05.md)) | sampled, ~every 300 s | 1,736 checks (2,644 unavailable) | not compared | 0 |
| [Nov 6–9 window](observation-window-audit-2026-11-10.md) | 15,634,038–15,725,032 | 90,978 | 90,982 heights with a success | 0 |
| [Nov 12 live hour](live-hour-observation-audit-2026-11-12.md) | 15,811,303–15,812,568 | 1,266 | 1,266 | 0 |
| [Nov 12–15 window](validator-observation-audit-2026-11-20.md) | 15,815,626–15,906,676 | 91,006 + 45 rechecked here = 91,051 | 91,006 in the audit; 45 not retained | 0 |

The transaction-result comparisons cover `code`, `data`, `gas_wanted` and
`gas_used` only. No mismatch has been recorded in any retained audit.

### Rust builds

| Period | Build |
|---|---|
| Oct 19–Nov 5 | Hourly auto-updater; 35 deployments in the 7 days to Nov 5. Each commit is not recorded in docs |
| Nov 5 23:12 UTC–Nov 9 07:34 UTC | `b459a6e041b2675582019cde5c75ecf2969318ba` (0.1.0-rc.5) |
| Nov 9–12 | Mainnet deployment at 07:34:38 UTC Nov 9 (height 15,727,376); later deployments include `3a6c71c`, `69df8af`, `a9f396b` and `8ffc98a`. Exact times per commit are not recorded here |
| Nov 12 02:00 UTC–Nov 15 05:29 UTC (audited hours) | `46acbcb516c3dab26cb5706c908dfe0207f27349`, SHA-256 `fdaf2930…a7c192` |
| After Nov 15 | Not recorded in docs |

## Not established

- Blocks proposed over the full history (above).
- Which build ran at each height, and the cause of each missed vote.
  The node logs and the Mac's deployment history are not reachable from here.
- Local transaction-result commitments for the 45 rechecked heights.
- Every-height app-hash or result comparison outside the retained windows.
  The periodic checker is a sample. Between the windows there is no
  per-height record.
- Double-sign evidence aged out before inclusion. The slash search finds only
  evidence that a block included. A `slash` event with reason `double_sign`
  would appear in the counts above.

## Method

Fetched 2026-11-25 01:59–02:22 UTC over HTTPS. Mainnet used
`https://rpc.celestia.pops.one`, `https://public-celestia-rpc.numia.xyz` and
REST `https://api.celestia.pops.one`. Mocha used `https://rpc-mocha.pops.one`,
`https://rpc.celestia-mocha.com` and REST `https://api-mocha.pops.one`.
Requests were paced at 0.3–0.7 s and backed off on HTTP 429/5xx. Raw results:
[mainnet.json](evidence/validator-track-record-2026-11-25/mainnet.json),
[mocha.json](evidence/validator-track-record-2026-11-25/mocha.json).

| Data | Query |
|---|---|
| State at the as-of height | REST `/cosmos/slashing/v1beta1/signing_infos/<valcons>`, `/cosmos/staking/v1beta1/validators/<valoper>`, `/cosmos/slashing/v1beta1/params`, `/cosmos/staking/v1beta1/pool`, each with header `x-cosmos-block-height: <as-of>` |
| Operator transactions | RPC `/tx_search?query="message.sender='<account>'"` |
| Missed votes | RPC `/block_search?query="liveness.address='<valcons>' AND block.height>=A AND block.height<=B"&per_page=1`, reading `total_count` |
| Slash events | The same with `slash.address`; the event at the jail height was read from `/block_results` |
| Active-set edges | RPC `/validators?height=H` (all pages) at heights around creation, jail and unjail |
| Miss run start | Binary search for the lowest A where the `liveness` count in A..jail height equals the number of heights |
| Signatures and proposers | RPC `/commit?height=H`: `block_id_flag` for the consensus address and `header.proposer_address` |
| 45-height recheck | RPC `/commit?height=H+1` from three providers, compared with `execution.app_hash` and `execution.block_hash` in the failed rows |

The data are reference RPC and REST assertions. Signatures were not verified
cryptographically, but independent providers agree on every compared value.
