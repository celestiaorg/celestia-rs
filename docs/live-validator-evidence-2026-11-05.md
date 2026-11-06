# Mac Mini validator evidence — 2026-11-05

Collected approximately **23:16–23:40 UTC** on 2026-11-05. This is a
dated observation, not a claim about current service state. Both original
validator processes remained running at the final check. No gate is closed
by this report. Source: local processes, installed non-secret configuration,
launchd, RPCs, Prometheus and node/monitoring logs; bulk responses were not
retained as repository artifacts.

## Identity and current health

Paths use `$HOME/.celestia-rs` as `H`. RPC/metrics bind to loopback; P2P
binds to all interfaces. Public identities came from staking/RPC data.

| Field | Mocha | Mainnet |
|---|---|---|
| Verified chain ID | `mocha-5` | `celestia` |
| Operator | `celestiavaloper13v73gcz3k923ajt23alr5h3wuvsgqxu3jag0sd` | `celestiavaloper16v6qxgj2w996jt49tngf9fkvxw6j7u0uh6d9ec` |
| Consensus hex | `64910794C3B7404929AB3CF18063A6B8CD3032D7` | `7FC8073F6F0DE936593BBBF6E6EBE37E17487615` |
| Consensus Bech32 | `celestiavalcons1vjgs09xrkaqyj2dt8nccqcaxhrxnqvkhqwen2s` | `celestiavalcons10lyqw0m0ph5nvkfmh0mwd6lr0ct5sas4ugtync` |
| Home / executable | `H/mocha` / `H/bin/celestia-rs` | `H/mainnet` / `H/bin/celestia-rs-mainnet` |
| System launchd label | `org.celestia.celestia-rs` | `org.celestia.celestia-rs-mainnet` |
| PID / start UTC, Nov 5 | 63147 / 22:54:19 | 68823 / 23:12:47 |
| RPC / metrics / P2P ports | 26657 / 26660 / 26656 | 26757 / 26760 / 26756 |
| Log under `~/Library/Logs/` | `celestia-rs.log` | `celestia-rs-mainnet.log` |
| Independent RPC used | `https://rpc.celestia-mocha.com` | `https://rpc.celestia.pops.one` |
| Paired local/public query UTC, Nov 5 | 23:31:11 | 23:31:36 |
| Local / public height | 2,354,577 / 2,354,577 | 15,626,209 / 15,626,209 |
| Local/public block time UTC, Nov 5 | 23:31:07.191654551 | 23:31:32.876392561 |
| Height lag / catching up | 0 / false | 0 / false |
| Peers at 23:17:56 | 10 | 9 |
| Direct staking query | bonded, unjailed | bonded, unjailed |
| Voting power / tokens (utia) | 1,010,001 / 1,010,001,000,000 | 56 / 56,303,623 |
| Data allocated (`du -sk`, ~23:18) | 6,847,096 KiB | 8,032,560 KiB |
| RSS / instantaneous CPU at 23:40 | 687,200 KiB / 0.3% | 1,014,672 KiB / 0.8% |

Both running builds: **0.1.0-rc.5**, commit
`b459a6e041b2675582019cde5c75ecf2969318ba`, **release** profile,
`rustc 1.99.1 (58cf8f9c3 2026-10-13) aarch64-apple-darwin`, app version
10, compatibility target celestia-app v10.5.0 / celestia-core v0.42.3.
Both executable SHA-256 values:

```text
fb66fba07c468595cc2801c9355054d8518149f4a62bb0ce78caac4d1d76efde
```

`lsof` loaded executable inodes matched the installed paths' `stat` inodes:
Mocha 24165340, Mainnet 24168243, each 62,869,728 bytes. This distinguishes
the loaded executables from a later atomic replacement. Both `.prev`
binaries were older commit `dbee4cda55343bc43550e1f82fb2640737da81d2`,
SHA-256 `b310062dbaf3e422b36f5c7ed3b5a1d1888d5465c3864aa2198e7a165d4a555e`.
No newer replacement was observed at the active installed paths.

Shared volume: 114,503,968 KiB available (~109.2 GiB), `df` capacity 47%.
Both launchd jobs were running. Mainnet retained historical last exit
reason `OS_REASON_CODESIGNING`; no matching panic text was found in recent
node logs. Individual termination causes were not independently established.

The installed Mainnet monitoring reference was
`https://celestia-rpc.publicnode.com`, which returned HTTP 403 during this
investigation. Polkachu also returned 403. Pops answered with chain ID
`celestia`; the Mocha reference answered with `mocha-5`.

## Consensus signing, separate from proposals and Fibre

Direct slashing queries at **23:40:22 UTC**:

| Field | Mocha | Mainnet |
|---|---|---|
| Missed blocks / signed-blocks window | 26 / 10,000 | 86 / 10,000 |
| Index offset / start height | 494,452 / 1,860,316 | 496,778 / 15,129,613 |
| Jailed until | `2026-10-20T11:06:44.728991637Z` | `2026-10-20T11:19:41.843285071Z` |
| Tombstoned | false | false |
| Minimum signed per window | 0.001 | 0.001 |

The missed-block count is a **rolling window**, not lifetime uptime.
At ~23:19 the counts were 25 and 87. Expired `jailed_until` alone does not
prove unjailed status; the direct staking query does.

Independent public `/commit` samples (inclusive heights), counting this
validator's `block_id_flag=2` as signing:

| Sample | Mocha | Mainnet |
|---|---|---|
| Heights | 2,354,476–2,354,575 | 15,626,108–15,626,207 |
| First block UTC, Nov 5 | 23:26:14.649540658 | 23:26:45.370513891 |
| Last block UTC, Nov 5 | 23:31:01.506167688 | 23:31:27.164200958 |
| Signed / missed | 99 / 1 (99%) | 100 / 0 (100%) |
| Missed height | 2,354,540 | none |
| Nil signatures / query errors | 0 / 0 | 0 / 0 |
| Proposals (header proposer) | 0 | 0 |

These short samples are not a 72-hour signing audit. No Fibre endorsement
data was collected or included in consensus-signing percentages.

## Historical coverage and incidents

History endpoint: **2026-11-05T23:19:45.852Z**. The 72-hour window starts
Nov 2 at that UTC time; the seven-day window starts Oct 29.

| Evidence | Mocha: 72h / 7d | Mainnet: 72h / 7d |
|---|---|---|
| Node starts in logs | 28 / 35 | 28 / 35 |
| Failed metrics scrapes | 1/17,280 / 1/40,319 | 61/17,280 / 75/40,320 |
| Collector RPC-unavailable samples | 2 / 4 | 57 / 73 |
| Catching-up samples | 5 / 5 | 12 / 14 |
| Available block-age <=60s samples | 17,278/17,278 / 40,316/40,316 | 17,220/17,223 / 40,244/40,247 |
| Observed local missed-counter increments | 223 / 788 | 295 / 598 |
| Local missed-counter resets | 26 / 33 | 26 / 33 |
| Height advance | 90,492 / 211,196 | 90,629 / 211,863 |
| Approximate local signing estimate | 99.754% / 99.627% | 99.674% / 99.718% |
| Double-sign-guard refusals | 0 / 1 | 0 / 0 |
| Matching panic, app-hash mismatch or snapshot-failure log patterns | 0 / 0 | 0 / 0 |

Local signing estimates are `100 * (1 - observed_misses / height_advance)`;
resets, downtime and missing samples can hide increments. They are not
authoritative on-chain percentages. Block age <=60s is a near-tip proxy;
historical independent height lag was not retained. Missing samples are
unknown time. Collector history has all 17,280 / 40,320 expected scrapes;
collector timestamp age never exceeded 35.718 seconds. Mocha's node scrape
history has one additional missing scrape over seven days.

The installed collector inferred jail status from tombstone/`jailed_until`,
unlike the current repository's staking-aware implementation. Its historical
`jailed=0` cannot establish absence of jail/unjail events. No recent
jail/unjail log events were found. Node logs cover Oct 15 through collection
for Mocha and Oct 19 through collection for Mainnet.

All incident times below are UTC; ranges are first/last matching observations.
Recurring-event rows span multiple events, not one continuous outage.

| Incident | First/last evidence |
|---|---|
| Mocha restarts, 35 in 7d | First stop/start Oct 31 00:49:49.649–00:49:50.483; latest Nov 5 22:54:19.510–22:54:20.312 |
| Mainnet restarts, 35 in 7d | First stop/start Oct 31 01:07:39.700–01:08:10.249; latest Nov 5 23:12:17.233–23:12:47.591 |
| Deployment history | Updater log: 35 deployments, 70 subsequent healthy results in 7d |
| Mocha guard refusal | Oct 31 04:11:22.088532, height 2,179,423, round 0, `conflicting data`; one occurrence |
| Mocha refusal alert firing | Oct 31 04:12:13–04:26:43 |
| Both AppHashCheckFailing alerts | Oct 29 23:20:00–Oct 31 18:14:00; already firing at window start (left-censored) |
| Later Mainnet failed parity checks | Nov 3 13:46:32 and Nov 5 01:01:42; failures to check, not mismatches |
| Mocha catch-up | Nov 4 06:33:19–06:33:34; Nov 5 19:13:35–19:14:05 |
| Mainnet catch-up | Oct 31 04:29:46–04:30:01; Nov 3 03:03:33–03:03:48 and 16:08:03–16:08:18; Nov 4 03:10:19–03:10:34, 17:18:34–17:18:49 and 22:20:49–22:21:04; Nov 5 02:23:04–02:23:19 |
| Mainnet block age >60s | Single samples Nov 3 16:08:18, Nov 4 17:18:49, Nov 5 02:23:19 |
| Mocha peers <3 | Single samples Nov 4 22:02:17; Nov 5 04:46:32 and 19:13:32 |
| Mainnet peers <3 | Single samples Nov 2 19:58:19; Nov 3 21:28:50; Nov 4 01:49:35, 13:57:50 and 18:38:50; Nov 5 23:12:51 |
| Mocha ValidatorMissingBlocks firing | Oct 30 11:15:58–11:17:43 and 11:43:43–11:44:58; Nov 1 11:04:13–11:04:43 |
| Mainnet ValidatorMissingBlocks firing | Oct 31 01:08:43–01:17:28 and 04:30:13–04:38:58; Nov 2 23:40:28–23:41:43; Nov 3 07:25:28–07:34:28 and 21:29:13–21:35:28; Nov 4 05:34:43–05:39:43 and 07:00:28–07:00:43; Nov 5 03:49:43–03:52:43 and 18:11:43–18:13:13 |

The longest available block-age <=60s sample sequence in seven days was
Mocha Oct 31 23:54:02–Nov 5 20:33:20 (116.65h), and Mainnet Nov 1
00:11:17–Nov 2 16:37:03 (40.43h). These sequences do not exclude brief
restarts between scrapes. **The latest 72 hours do not establish an
uninterrupted three-day Mainnet at-tip run.**

## Parity evidence

Fresh successful installed `check-apphash.sh` comparisons:

| Network | Executed height | Check UTC, Nov 5 | Matching hash |
|---|---|---|---|
| Mocha | 2,354,584 | 23:31:32.456–23:31:36.638 | `167C4C8BED48364295A89AD030DDA8F14813AEAFED48E44F9FDD1EE38698CA30` |
| Mainnet | 15,626,217 | 23:32:00.123–23:32:04.527 | `06CB9421E34FD76117D63244D3D855EF1E73CD14BF79C4D26E6CEF04D7F38D8B` |

Distinct observed check timestamps, deduplicated from Prometheus:

| Window | Mocha: success / unavailable / mismatch | Mainnet: success / unavailable / mismatch |
|---|---|---|
| 72h | 843 / 0 / 0 | 841 / 2 / 0 |
| 7d | 1,465 / 425 / 0 | 1,461 / 429 / 0 |
| All retained | 1,789 / 2,591 / 0 | 1,736 / 2,644 / 0 |

Retained parity scrape history spans Oct 19 14:35:11–Nov 5 23:19:35 UTC.
Mocha successes span Oct 19 14:35:06–Nov 5 23:18:00; unavailable checks
span Oct 20 03:14:05–Oct 31 18:13:56. Mainnet successes span Oct 19
14:35:10–Nov 5 23:18:04; unavailable checks span Oct 20 03:19:38–Nov 5
01:01:42. Zero observed mismatch results does not turn unavailable checks
into successful comparisons.

These are **periodic samples**, nominally every 300s: local execution H
versus the reference header H+1. Prometheus stores result/timestamp but
not checked height/hash. Counts describe checks observed by scrapes, not
an immutable execution ledger. The checker does not compare transaction
results. No continuous live-node transaction-result comparison evidence
was established. The older cloud replay documented in M13 is separate
historical evidence, not a new verification of these processes.

## Operations and live-node gates

Prometheus PID 328, Grafana PID 321 and node-exporter PID 324 were running;
all four scrape targets were healthy at ~23:18 and no alerts were firing.
Installed monitoring root: `H/monitoring`; Prometheus config/rules/data
under `prometheus/`, retention 90d, scrape/evaluation 15s, HTTP loopback
port 9090. Watchdog (60s), collector (30s) and apphash (300s) launchd timer
run counts advanced from 22,517/43,330/4,048 to 22,538/43,371/4,052;
last exit codes were 0. Idle between timer runs is expected.

Grafana logged forwarding alerts to its local notifier during the Mocha
refusal period. **Email delivery was not established.** Credentials and
alerting environment files were not read. Existing hourly auto-update
remained installed; no live configuration was changed by the docs update.

| Gate | Proven | Still unknown or unmet |
|---|---|---|
| M13 | Current Mocha validation and spot parity; prior accepted Oct 16–17 soak remains historical evidence | No new clean soak on this candidate; seven-day refusal; no proof of Mac live crossing of historical mainnet upgrade |
| C1.1 | Mainnet at tip on v10 at observation; periodic parity | Latest 72h interrupted; every tx result matching unproven |
| C1.5 | Distinct public identities, homes/ports; watchdog, monitoring and alert evaluation; written halt response | Signer exclusivity/provenance, current custody and email delivery; installed jail metric insufficient |
| F1.8 | Current traffic executed and sampled hashes match | Whole-era replay, every-height hash/tx-result comparison and required historical continuity |

Tracked follow-ups:

- [#297](https://github.com/celestiaorg/celestia-rs/issues/297): installed monitoring drift, staking jail status and RPC references.
- [#298](https://github.com/celestiaorg/celestia-rs/issues/298): auditable parity receipts and transaction-result evidence.
- [#299](https://github.com/celestiaorg/celestia-rs/issues/299): exact-candidate three-day qualification and restart diagnosis.
- [#300](https://github.com/celestiaorg/celestia-rs/issues/300): alert delivery verification.
- [#301](https://github.com/celestiaorg/celestia-rs/issues/301): signing-key provenance and custody attestation.
- [#302](https://github.com/celestiaorg/celestia-rs/issues/302): remaining F1.8 replay coverage.

## Collection method and limitations

Commands included `ps`, `launchctl print system/<label>` (whitelisted
state fields), `lsof -a -p <pid> -d txt -F nDis`, `stat`, `shasum -a 256`,
the exact node binary's `version --long`, `du -sk <home>/data`, `df -k`,
and `query staking validator`, `query slashing params` and
`query slashing signing-info`, each with explicit `--node <local-rpc>`.
The installed check script was invoked with explicit local/public RPCs.
Non-secret config/plist fields and matching log patterns were inspected.

HTTP GETs: `/status`, `/net_info`, node `/metrics`, public
`/commit?height=H` over the exact sample ranges above, local `/abci_info`
and public `/block?height=H+1`. Prometheus GETs used `/api/v1/targets`,
`/api/v1/alerts` and `/api/v1/query` with `time=1793920785.852`.
Raw matrix expressions used `[7d]` on:

```promql
up{job="celestia-rs"}
celestia_rs_rpc_up
celestia_rs_catching_up
cometbft_p2p_peers
celestia_rs_latest_block_time_seconds
celestia_rs_collector_timestamp_seconds
cometbft_consensus_validator_missed_blocks
cometbft_consensus_latest_block_height
celestia_rs_apphash_match
celestia_rs_apphash_check_timestamp_seconds
ALERTS{alertstate="firing"}
celestia_rs_validator_jailed
celestia_rs_validator_tombstoned
```

Parity match/timestamp also used `[90d]`; aggregates included
`min_over_time`, `max_over_time`, `count_over_time`, `avg_over_time`,
`increase` and `resets` over `[72h]`/`[7d]`. Local counter increments were
also summed directly with reset handling. Missing samples were not
classified as healthy. No secret files or signing state were read/copied.

Collection deviation: a version-inspection filter accidentally invoked
`celestia-rs-auto-update.sh.bak` and `.pre-unjail` with `version --long`.
Both returned `wait: CI for b5f78f1 not finished (30 of 30 runs pending)`
before deployment/restart. Their pre-CI code touches updater bookkeeping,
creates/removes a lock and fetches Git metadata. This violated the original
read-only constraint; it does not justify calling the collection wholly
read-only. Both original validator PIDs remained running at the final check.
