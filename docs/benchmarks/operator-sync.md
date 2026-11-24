# Operator sync comparison

The Go reference is [celestia-app v11.0.0](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0), commit `58be8ae90d00518800df5506b3a1e988f009c1c6`. Record the Rust commit and both executable checksums. No v11 cloud measurements are available yet. The [October Mac Mini measurements](README.md#results-mac-mini-m4-2026-10-20-b1-reference) remain a separate v10.4.0 series.

## What to time

Run non-validating consensus full nodes with DA bridge services disabled on both sides. Use the same chain ID, verified genesis digest, peer set, pruning and transaction indexing policy. Include any historical binaries and upgrade restarts needed for genesis replay. Verify that archive peers actually serve height 1 through the target and that each implementation supports those historical versions. A missing interval or unsupported upgrade is a recorded failure.

| Starting point | Start the timer | Finish |
| --- | --- | --- |
| State sync | First process launch with empty data, identical trust inputs, snapshot height and providers | First verified tip |
| Genesis block sync | First process launch with verified genesis installed and empty data | First verified tip |

Provision identical dedicated Linux VMs and empty data disks in the same zone. Start the pair together. Record the machine and disk types, vCPU count, RAM, disk capacity, OS, network, software and configuration. Keep providers outside the measured VMs. Record actual snapshot height, restore completion and first post-restore commit separately. Discard a paired comparison if different snapshots were restored.

Poll local and independent reference RPCs every five seconds. Record monotonic elapsed time, local height, reference height and catching-up state. Count a node as at tip when catching_up is false and it is within one block of the reference on two consecutive polls with advancing local heights. Verify the chain ID and local block ID against the reference at the common height. Use the first qualifying poll as finish time after confirmation. Also record time to a common fixed checkpoint for pairs whose live tips differ. Timeouts, divergent hashes and interrupted runs get the last verified height and reason. Do not extrapolate a finish time.

## Resources and reporting

- Place the node and its child processes in a dedicated cgroup v2 service. Report peak cgroup memory (includes charged file cache), sampled process-tree RSS with its sampling interval, and steady-state memory after 30 minutes at tip. Preserve the cgroup peak before stopping the service, including across any upgrade restarts.
- Use an empty, dedicated data filesystem. Report allocated bytes for the whole node home, block store, application state and snapshots. Sample filesystem used bytes during sync for a high-water mark. Report the initial baseline and retained height range. For an equal-height comparison, use a matched checkpoint run stopped gracefully at the same height and take the disk measurements after shutdown.
- Record elapsed times, peak RAM, steady-state RAM, peak disk and final allocated bytes in original units for each implementation and trial. Keep raw timestamped measurements, configuration, peer IDs and the verification receipt. Run three cold pairs if the budget allows, publishing each pair and the median. Label a single pair as one observation. Record failed attempts alongside completed runs.
- Benchmark the integrated DA bridge separately against the Go consensus-plus-bridge stack, with matching DA retention and client workload. Consensus-only measurements do not quantify the resource cost of serving DA.
