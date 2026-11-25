# Operator sync comparison

The Go reference is [celestia-app v11.0.0](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0), commit `58be8ae90d00518800df5506b3a1e988f009c1c6`. Record the Rust commit and both executable checksums. The [2026-11-25 runs](operator-sync-2026-11-25.md) are the first v11 cloud measurements: one state-sync observation for each implementation. The [October Mac Mini measurements](README.md#results-mac-mini-m4-2026-10-20-b1-reference) remain a separate v10.4.0 series.

## What to time

Run non-validating consensus full nodes with DA bridge services disabled on both sides. Use the same chain ID, verified genesis digest, peer set, pruning and transaction indexing policy. Include any historical binaries and upgrade restarts needed for genesis replay. Verify that archive peers actually serve height 1 through the target and that each implementation supports those historical versions. A missing interval or unsupported upgrade is a recorded failure.

| Starting point | Start the timer | Finish |
| --- | --- | --- |
| State sync | First process launch with empty data, identical trust inputs, snapshot height and providers | First verified tip |
| Genesis prefix | First process launch with verified mainnet genesis installed and empty data | Execution of a fixed height H, verified against the app hash in signed header H+1 |

Provision identical dedicated Linux VMs and empty data disks in the same zone. Start the pair together. Record the machine and disk types, vCPU count, RAM, disk capacity, OS, network, software and configuration. Keep providers outside the measured VMs. Record actual snapshot height, restore completion and first post-restore commit separately. Discard a paired comparison if different snapshots were restored.

Poll local and independent reference RPCs every five seconds. Record monotonic elapsed time, local height, reference height and catching-up state. Count a node as at tip when catching_up is false and it is within one block of the reference on two consecutive polls with advancing local heights. Verify the chain ID and local block ID against the reference at the common height. Use the first qualifying poll as finish time after confirmation. Also record time to a common fixed checkpoint for pairs whose live tips differ. Timeouts, divergent hashes and interrupted runs get the last verified height and reason. Do not extrapolate a finish time.

## Mainnet prefix and optimization trials

Start with H = 10, the height covered by the retained v1 conformance window. On 2026-11-24 a local preflight with the v0.1.0-rc.9 Linux release and the canonical mainnet genesis (SHA-256 `9727aac9bbfb021ce7fc695a92f901986421283a891b89e0af97bc9fad187793`) stopped at InitChain with `gov genesis params are nil`. The genesis has separate deposit, voting and tally parameters. No block executed. On 2026-11-25 the node learned to read the genesis app version 1 and to store these gov parameters as cosmos-sdk v0.46 does. InitChain now stops later, at IBC: `invalid ibc channel genesis state parameters: upgrade timeout timestamp invalid: 0: upgrade timeout is invalid`. The node has no app version 1 state machine yet (for example, it lacks the v1 `qgb` store), so no block can match mainnet. The [historical replay test](../../conformance/README.md#historical-mainnet-replay) records the same issue and provides independent block and post-state hashes for the first ten heights.

Require historical initialization and matching commitments before timing this prefix. Extend H only after a successful run; hold H and inputs constant across optimization trials. Time initialization and subsequent block execution separately. Profile baseline CPU, storage I/O and peer wait time before changing the implementation. Compare the baseline and candidate on fresh volumes with identical verification enabled and report repeated durations and hash checks. A later checkpoint is a separate series with its own start height.

For cloud trials, preprice both machines, disks, IPs and network charges together. Bound runtime and traffic, use disposable auto-deleted disks, and include cleanup headroom within the $10 ceiling for each comparison. A billing alert alone cannot enforce that ceiling.

## Resources and reporting

- Place the node and its child processes in a dedicated cgroup v2 service. Report peak cgroup memory (includes charged file cache), sampled process-tree RSS with its sampling interval, and steady-state memory after 30 minutes at tip. Preserve the cgroup peak before stopping the service, including across any upgrade restarts.
- Use an empty, dedicated data filesystem. Report allocated bytes for the whole node home, block store, application state and snapshots. Sample filesystem used bytes during sync for a high-water mark. Report the initial baseline and retained height range. For an equal-height comparison, use a matched checkpoint run stopped gracefully at the same height and take the disk measurements after shutdown.
- Record elapsed times, peak RAM, steady-state RAM, peak disk and final allocated bytes in original units for each implementation and trial. Keep raw timestamped measurements, configuration, peer IDs and the verification receipt. Run three cold pairs if the budget allows, publishing each pair and the median. Label a single pair as one observation. Record failed attempts alongside completed runs.
- Benchmark the integrated DA bridge separately against the Go consensus-plus-bridge stack, with matching DA retention and client workload. Consensus-only measurements do not quantify the resource cost of serving DA.

## Running the campaigns

[`operator-benchmark.yml`](../../.github/workflows/operator-benchmark.yml) runs these comparisons without SSH. Dispatch it from `main`. `provision` creates every pair in one call: two identical `n2-standard-8` VMs with an empty 200 GB `pd-balanced` data disk each, in `us-central1-a`. Each VM deletes itself at its maximum run duration. Each VM runs [`scripts/bench/vm.sh`](../../scripts/bench/vm.sh) from its startup script. It builds its executable outside the timed interval: celestia-rs from `rust_commit` with `--features da-light`, or celestia-app `v11.0.0` with `make build` (multiplexer). Then it starts one node as a transient systemd service in `benchnode.slice` and polls it. Both sides get the same genesis, seeds, peers, trust inputs, pruning (`custom`, 100 versions, interval 10, 3000 retained blocks), snapshots off and no transaction index. The VM caps its egress at 4 Mbit/s to bound network charges.

| Campaign | Dispatch inputs | VM lifetime | Worst-case price |
| --- | --- | --- | --- |
| Genesis prefix | `action=provision campaign=genesis-prefix height=10` and, if needed, archive `peers` | 3 h | $3.89 |
| State sync | `action=provision campaign=state-sync pairs=1` (up to 3 concurrent cold pairs) | 4 h | $5.18 per pair |
| DA bridge | `action=provision campaign=bridge pairs=1` | 5 h | $6.48 per pair |

`rust_commit` defaults to the dispatching commit. Extend `height` only after a successful run. The workflow's header comment has the price arithmetic. The job summary prints the run ID and the deletion time.

Genesis prefix polls every second until the local height reaches H. It waits for H+1, then compares the local header H+1 app hash with the reference RPC and runs `packaging/check-apphash.sh`. CometBFT serves RPC only after its ABCI handshake, so `rpc_up_s` includes InitChain on Go. `first_height_s` and `target_s` bound block execution at poll resolution. State sync polls every five seconds until the tip rule above holds. It then checks the chain ID and block ID at the local height and stays at tip for 30 minutes. A stopped service, no height change (30 minutes for genesis prefix, 60 for state sync) or the deadline ends the run with the last height and a reason.

The bridge campaign state-syncs both sides the same way. celestia-rs runs with its integrated bridge (`--bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen /ip4/0.0.0.0/tcp/2121`). The Go VM also builds celestia-node `v0.36.0` (commit `d5dfb21a0885622828e30e8e2daf50e5d1bb72b2`, built against celestia-app `v11.0.0`) and starts its bridge as a second service in the same slice once celestia-app is at tip. Both keep their default 169-hour DA retention. The 60-minute window starts at `window_start_s`. Take CPU seconds and window RAM from the polls after that point, and disk from the polls and the final `du`. A local `header.LocalHead` probe runs each minute (`da.tsv`). No client workload is generated, so label these results "live mainnet traffic only".

Each VM stops its node, preserving the cgroup peak first, and publishes guest attributes under `bench/`: `status`, `summary` (JSON) and `raw-0`… (compressed polls, logs, configuration and `du` per data directory). Dispatch `action=collect` with the same `campaign`, `pairs` and `run_id` before the deletion time. Its job summary shows each VM's status; if one does not read `done-…` yet, dispatch it again later. The artifact `operator-bench-<campaign>-<run_id>` keeps the results for 90 days and lists pair validity. A pair that restored different snapshots is `discard:different_snapshots`. Then dispatch `action=delete` to stop charges early. Record each run, including failures, in a dated `docs/benchmarks/operator-sync-<date>.md` with its run ID and artifact. Do not add it to this protocol.
