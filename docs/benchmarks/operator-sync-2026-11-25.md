# Operator sync runs, 2026-11-25

These are the first cloud runs of the [operator comparison protocol](operator-sync.md). Run 2 is one state-sync observation for each side from a pair that restored different snapshots. Run 3 is two valid state-sync pairs. Run 4 is two attempts at the DA bridge comparison. The [summary](#summary-of-the-valid-pairs) compares the two valid pairs.

Go is celestia-app `v11.0.0` (commit `58be8ae90d00518800df5506b3a1e988f009c1c6`) built with the multiplexer. Each VM is `n2-standard-8` (8 vCPU, 32 GB) in `us-central1-a` with Ubuntu 24.04, kernel `7.0.0-1017-gcp`, an empty 200 GB `pd-balanced` data disk, BBR, and egress capped at 4 Mbit/s. Both nodes ran with pruning `custom` (100 versions, interval 10), 3000 retained blocks, snapshots off and no transaction index. DA bridge services were off in runs 1 to 3 and on in run 4.

## Run 1: failed, no measurement

Provisioning run [45555875126](https://github.com/celestiaorg/celestia-rs/actions/runs/45555875126), celestia-rs `d5970b85aba8f8b8b7d3e585576414a3a4c7cdd4`. Both sides built and both failed before any sync. Neither failure came from a node.

| Side | Last height | Reason |
| --- | --- | --- |
| celestia-rs | none | The benchmark script stopped its own node 0.5 s after start. Its error trap ran in a subshell on the first poll. |
| Go | none | `celestia-appd` exited with `BBR not enabled because output cubic`. The VM image defaults to cubic. |

[#584](https://github.com/celestiaorg/celestia-rs/pull/584) fixed both. The VMs were deleted after 40 minutes.

## Run 2: state sync to tip, one pair

Provisioning run [45562712986](https://github.com/celestiaorg/celestia-rs/actions/runs/45562712986) at 06:43 UTC, celestia-rs `cbbbe9a44b6de2cee0b0f430236ca30885ba0627`. Trust height 16,209,000, trust hash `A7E4ECD091E78D7085285B6456A3A308C8673EEC5A01B2604DD61F729B78CB86`, the same on both sides. Both runs ended `ok`. Pair verdict: `discard:different_snapshots`.

| Measure | Go v11.0.0 | celestia-rs |
| --- | ---: | ---: |
| Executable SHA-256 | `003b1d99…9ce452` | `a9fc5284…6cb78f` |
| Snapshot restored | 16,210,500 | 16,212,000 |
| Data disk starts to grow | 5.6 s | 3,003.1 s |
| First height served after restore | 1,412.5 s | 3,175.7 s |
| First verified tip | 1,546.8 s | 3,308.4 s |
| Block ID check at tip | match at 16,212,754 | match at 16,213,516 |
| Peak cgroup memory | 13,332,463,616 B | 6,044,954,624 B |
| Peak sampled RSS (5 s polls) | 8,894,400 KiB | 2,354,360 KiB |
| RSS, last 5 minutes of the 30 minutes at tip (median) | 5.79 GiB | 1.42 GiB |
| cgroup memory, same interval (median) | 9.53 GiB | 3.16 GiB |
| Node home after shutdown | 5,238,022,144 B | 1,859,239,936 B |
| Filesystem high-water mark | 5,367,758,848 B | 3,402,043,392 B |
| CPU time, whole run | 2,458 s | 981 s |

Times are monotonic seconds from the first process launch. Build time is not included. The cgroup figures include charged file cache. The filesystem figures include files outside the node home, and the two VMs started from different baselines (1,227,018,240 B on Go, 176,099,328 B on celestia-rs).

What this run shows:

- **Time to tip:** Go was at tip in 25.8 minutes. celestia-rs took 55.1 minutes, about 2.1 times as long.
- **Where the time went on celestia-rs:** for the first 50 minutes the node wrote nothing to disk and held at most 0.6 GiB of RSS. The restore then took at most 173 s (from 3,003 s to 3,176 s). Go restored for about 23 minutes without a wait. The wait, not the restore, is the difference.
- **Cause of the wait:** not established. This run published only the polls, because the full node log was too large for the guest attributes. In run 1 the celestia-rs log showed seeds rejected as `peer is only using legacy propagation` and `Couldn't connect to any seeds`, so slow snapshot discovery from a small peer set is the first thing to check. The script now publishes a log excerpt for this.
- **Memory:** celestia-rs peaked at 45% of Go's cgroup memory and 26% of Go's RSS. At tip it held 1.42 GiB RSS against 5.79 GiB.
- **Disk:** the celestia-rs home was 35% of Go's after shutdown. The retained heights differ by 1,500 because the snapshots differ.
- **CPU:** celestia-rs used 40% of Go's CPU time, over a run that was 29 minutes longer.

Limits: one observation for each side, live mainnet peers, different snapshots, and a 4 Mbit/s egress cap on both. The VMs were deleted after collection at 08:22 UTC.

## Run 3: state sync to tip, two valid pairs

Provisioning run [45579019615](https://github.com/celestiaorg/celestia-rs/actions/runs/45579019615) at 08:45 UTC, celestia-rs `4e6d139cb1317e20a065c38600537a67b3b7339b` (executable SHA-256 `337698e8…769f4a`). Two cold pairs ran at the same time. Trust height 16,211,000 on all four VMs. All four runs ended `ok` and restored snapshot 16,213,500, so both pairs are `valid`.

| Measure | Go, pair 1 | celestia-rs, pair 1 | Go, pair 2 | celestia-rs, pair 2 |
| --- | ---: | ---: | ---: | ---: |
| First height served after restore | 1,422.7 s | 349.1 s | 1,930.1 s | 376.4 s |
| First verified tip | 1,534.9 s | 481.6 s | 2,051.9 s | 520.2 s |
| Block ID check at tip | match at 16,215,315 | match at 16,215,091 | match at 16,215,498 | match at 16,215,095 |
| Peak cgroup memory | 11,733,966,848 B | 6,020,755,456 B | 12,493,234,176 B | 5,961,822,208 B |
| Peak sampled RSS | 7,597,960 KiB | 2,243,168 KiB | 8,312,244 KiB | 2,325,792 KiB |
| RSS, last poll after 30 minutes at tip | 5.77 GiB | 1.32 GiB | 5.52 GiB | 1.31 GiB |
| Node home after shutdown | 5,177,733,120 B | 1,813,573,632 B | 5,199,388,672 B | 1,780,957,184 B |
| Filesystem high-water mark | 5,395,742,720 B | 3,586,715,648 B | 5,422,297,088 B | 3,316,350,976 B |
| CPU time, whole run | 2,403 s | 882 s | 2,448 s | 942 s |

celestia-rs reached the tip 3.2 times as fast as Go in pair 1 and 3.9 times as fast in pair 2.

The celestia-rs logs give the phases. In pair 1 the node started discovery at 08:57:22, found the snapshot at 09:00:01 (159 s), started the restore at 09:00:12 and completed it at 09:02:59 (167 s for 69 chunks). Pair 2 found the same snapshot at 09:00:01 after 186 s and restored in 161 s. Both nodes found the snapshot in the same second, although they started 27 s apart.

The polls of this run are not in the artifact. Guest attributes kept only the first two raw chunks for each VM, and the archive put the node log first. The table uses the summaries, which hold the peaks and the last poll. The script now limits the raw data to two chunks.

## Run 4: integrated DA bridge against celestia-app plus a celestia-node bridge

celestia-rs ran with its integrated bridge (`--bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen /ip4/0.0.0.0/tcp/2121`). The Go VM ran celestia-app `v11.0.0` and a celestia-node `v0.36.0` bridge (commit `d5dfb21a0885622828e30e8e2daf50e5d1bb72b2`) as a second service, started when celestia-app was at tip. Both sides were set to state-sync first. The measured window is 60 minutes from the first verified tip, under live mainnet traffic only. No client workload ran.

**The two sides did not do the same work in the window.** The celestia-node bridge started its local head near 16,071,000 and filled in about 133,000 to 149,000 older heights during the window; it wrote 33 to 49 GB. The celestia-rs bridge had its local head at the consensus height from the first probe and stored only the heights after its snapshot. So this run compares a celestia-node bridge in catch-up with a celestia-rs bridge that serves from its snapshot height. It does not compare equal DA retention, and it is not a steady-state figure for either side.

### Attempt A: celestia-rs failed

Provisioning run [45598700560](https://github.com/celestiaorg/celestia-rs/actions/runs/45598700560) at 10:50 UTC, celestia-rs `bd37709ad3b54b120f2ff7952c67c01db518519c`, trust height 16,214,000.

celestia-rs ended `failed` with reason `stalled`: no height for 60 minutes. Its log excerpt shows:

- 11:01:08: `Discovering snapshots discovery_time=15s`.
- 11:01:57: `Discovered new snapshot height=11643500 format=3`. This is 4,570,500 heights below the trust height. It is the only snapshot the node discovered.
- No state sync line for the next 59 minutes.
- 12:01:05, when the script stopped the node: `failed to fetch and verify app hash ... no witnesses connected`, then `Snapshot rejected height=11643500 format=3`.
- In the same hour: 3,075 `Stopping peer for error ... err="EOF"` lines and 92 `Added peer` lines.

Go ended `ok` (snapshot 16,216,500, tip at 1,805.7 s, block ID match). Its figures are in the table below.

### Attempt B: both sides completed

Provisioning run [45612622177](https://github.com/celestiaorg/celestia-rs/actions/runs/45612622177) at 12:19 UTC, the same celestia-rs commit (executable SHA-256 `27fb2f21…5ea1e8`), trust height 16,216,000. Both ended `ok` with a block ID match at tip. The state-sync snapshots differ (16,216,500 on Go, 16,218,000 on celestia-rs), so the pair verdict is `discard:different_snapshots`.

| Measure | Go app + celestia-node bridge, attempt A | Go app + celestia-node bridge, attempt B | celestia-rs with bridge, attempt B |
| --- | ---: | ---: | ---: |
| First verified tip | 1,805.7 s | 1,190.2 s | 358.2 s |
| Peak cgroup memory, all services | 30,468,751,360 B | 27,747,430,400 B | 6,049,558,528 B |
| Peak cgroup memory by service | app 12.94 GB, bridge 22.14 GB | app 12.90 GB, bridge 20.86 GB | one process |
| RSS in the window, median / maximum | 8.75 / 12.64 GiB | 8.41 / 10.25 GiB | 1.41 / 1.48 GiB |
| cgroup memory in the window, median / maximum | 23.32 / 28.30 GiB | 20.90 / 25.79 GiB | 3.07 / 3.78 GiB |
| CPU time in the window | 7,400 s | 5,334 s | 990 s |
| Node home after shutdown | 5,192,212,480 B | 5,363,036,160 B | 1,881,800,704 B |
| Bridge store after shutdown | 48,762,679,296 B | 33,034,764,288 B | inside the node home |
| DA local head, first probe to last probe | 16,070,350 to tip | 16,071,046 to 16,204,358 | at the consensus height in every probe |

In attempt B, celestia-rs with its bridge used 17% of the RSS (median), 22% of the peak cgroup memory and 19% of the CPU time of the Go pair of processes. Read these with the note above: the Go bridge was in catch-up and held far more DA history.

The celestia-rs node in attempt B also logged 2,922 peer `EOF` lines against 93 `Added peer` lines in about an hour, so the peer disconnects are not specific to the failed attempt.

## Summary of the valid pairs

The protocol discards run 2 as a paired comparison, so the comparison uses the two valid pairs of run 3. Two pairs do not give a median of three; a third valid pair is still to run.

| Measure | Go v11.0.0, pair 1 / pair 2 | celestia-rs, pair 1 / pair 2 | celestia-rs / Go, pair 1 / pair 2 |
| --- | ---: | ---: | ---: |
| First verified tip | 1,534.9 / 2,051.9 s | 481.6 / 520.2 s | 0.31 / 0.25 |
| Peak cgroup memory | 11.73 / 12.49 GB | 6.02 / 5.96 GB | 0.51 / 0.48 |
| Peak sampled RSS | 7.25 / 7.93 GiB | 2.14 / 2.22 GiB | 0.30 / 0.28 |
| RSS after 30 minutes at tip (last poll) | 5.77 / 5.52 GiB | 1.32 / 1.31 GiB | 0.23 / 0.24 |
| Node home after shutdown | 5.18 / 5.20 GB | 1.81 / 1.78 GB | 0.35 / 0.34 |
| CPU time, whole run | 2,403 / 2,448 s | 882 / 942 s | 0.37 / 0.39 |

Run 2 stays on record as one observation for each side, outside this comparison. In it celestia-rs took 3,308.4 s to the tip and Go took 1,546.8 s. Its resource figures agree with the valid pairs.

Snapshot discovery explains the difference between run 2 and run 3 on celestia-rs. It took 159 s and 186 s in run 3 and about 50 minutes in run 2, where Go started its restore in the first seconds from an older snapshot (16,210,500). The run 3 logs show that both nodes found the snapshot in the same second.

Run 4 attempt A shows what happens in a slow case: the node discovered one snapshot, 4.6 million heights old, worked on it for 59 minutes and found no newer one. Run 2 kept no log, but its wait fits the same pattern. The observed cause is that celestia-rs takes up a very old snapshot and does not give it up for a long time. The fault in the code is not yet located. Of the five celestia-rs state syncs in these runs, three found a recent snapshot in under 4 minutes, one waited about 50 minutes and one did not sync in 60 minutes. This is the first thing to fix before a time to tip is quoted.

## Not run yet

- **Genesis prefix:** celestia-rs cannot run it. `InitChain` on the mainnet genesis stops at IBC, and there is no app version 1 state machine. See the [historical replay record](../../conformance/README.md#historical-mainnet-replay).
- **DA bridge with equal retention:** run 4 does not hold the retained DA history equal on both sides. A like-for-like figure needs both bridges to cover the same height range, and a matched client workload.
- **A third valid state-sync pair.**
