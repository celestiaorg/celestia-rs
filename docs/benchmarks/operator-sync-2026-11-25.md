# Operator sync runs, 2026-11-25

These are the first cloud runs of the [operator comparison protocol](operator-sync.md). They give one state-sync observation for each implementation. The pair restored different snapshots, so the protocol discards it as a paired comparison. The numbers below are single observations, not a median.

Go is celestia-app `v11.0.0` (commit `58be8ae90d00518800df5506b3a1e988f009c1c6`) built with the multiplexer. Each VM is `n2-standard-8` (8 vCPU, 32 GB) in `us-central1-a` with Ubuntu 24.04, kernel `7.0.0-1017-gcp`, an empty 200 GB `pd-balanced` data disk, BBR, and egress capped at 4 Mbit/s. Both nodes ran with pruning `custom` (100 versions, interval 10), 3000 retained blocks, snapshots off and no transaction index. DA bridge services were off.

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

Limits: one observation for each side, live mainnet peers, different snapshots, and a 4 Mbit/s egress cap on both. Three cold pairs and their median are still to run. The VMs were deleted after collection at 08:22 UTC.

## Not run yet

- **Genesis prefix:** celestia-rs cannot run it. `InitChain` on the mainnet genesis stops at IBC, and there is no app version 1 state machine. See the [historical replay record](../../conformance/README.md#historical-mainnet-replay).
- **DA bridge:** not run.
