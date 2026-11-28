# celestia-rs benchmarks

## Mainnet operator benchmarks

These runs compare celestia-rs with Go celestia-app v11.0.0 on paired cloud VMs. The DA bridge runs also use Go celestia-node v0.36.0.

| Report | What it measures |
| --- | --- |
| [State sync and DA bridge, November 26](operator-sync-2026-11-26.md) | Time to a verified tip, plus memory, CPU, and disk with both bridges serving the same reads. |
| [1,000-block consensus runs, November 26](operator-block-range-2026-11-26.md) | Snapshot discovery and restore, peak memory, and final node-home size on 32-vCPU VMs. |
| [First operator sync runs, November 25](operator-sync-2026-11-25.md) | Earlier sync and DA pairs, including failed attempts and the reasons they were excluded. |
| [Operator comparison protocol](operator-sync.md) | How to reproduce paired runs and verify the results. |

See the [summary charts](https://celestiaorg.github.io/celestia-rs/#benchmarks) and [hardware requirements](../hardware.md) for operator-facing results.

## Replay, storage, and memory

- [Replay and full-node benchmarks](replay-and-node-benchmarks.md) covers the Mac Mini M4 comparisons, Linux reruns, application-state disk usage, and proposal pipeline tuning. It includes [reproduction commands](replay-and-node-benchmarks.md#reproduce-mac-mini-or-any-machine).
- [Memory and allocator benchmarks](memory.md) covers long-running memory soaks, RSS charts, and the mimalloc comparison.
