# Operator block-range runs on validator hardware, 2026-11-26

These are the first runs of the `block-range` campaign of the
[operator comparison protocol](operator-sync.md). Each pair starts one
celestia-rs node and one Go node cold, state-syncs both, and then samples both
over the same 1,000 mainnet blocks. Both nodes halt at the same height.

The result is two valid pairs out of four attempts. Over the same 1,000 blocks
celestia-rs used 75% and 78% less peak process memory than Go, and 62% to 64%
less disk. celestia-rs restored a snapshot in under 2 minutes against about 15
minutes for Go. The total time to the tip was not consistently better, because
celestia-rs was slow to discover a snapshot in three of four runs.

## Implementations and environment

- celestia-rs commit `b8ae9ce115e9f6fb33b520ae259779098e464f02`, built on each
  VM with `rustc 1.100.0`.
- celestia-app `v11.0.0`, commit `58be8ae90d00518800df5506b3a1e988f009c1c6`.
- Cold `n2-highcpu-32` VMs in `us-central1-a`: 32 vCPUs, 31.3 GiB of RAM,
  minimum CPU platform Intel Ice Lake (GFNI, SHA-NI, AVX-512, VAES), four local
  NVMe SSDs in RAID 0, kernel `7.0.0-1018-gcp`. This matches the CPU and
  memory of the [recommended validator
  hardware](https://docs.celestia.org/operate/getting-started/hardware-requirements/).
- Consensus only: no DA bridge on either side. Egress limited to 4 Mbit/s.
  The runs were provisioned with `scripts/bench/cloud.sh provision`.

## Attempts

| Run | Pair | Range start | Snapshot (rs / Go) | Verdict |
| --- | --- | ---: | --- | --- |
| `20261126195144` | p1 | 16,260,996 | 16,258,500 / 16,258,500 | valid |
| `20261126221446` | p1 | 16,264,022 | n/a | discard: Go state sync stalled |
| `20261126221446` | p2 | 16,264,022 | 16,261,500 / 16,260,000 | discard: different snapshots |
| `20261126232315` | p1 | 16,265,467 | 16,261,500 / 16,261,500 | valid |

In the stalled attempt the Go node logged `Discovering snapshots` and then made
no state sync progress for one hour, so the harness failed it at its stall
limit. In the `different_snapshots` attempt celestia-rs discovered its first
snapshot after the Go node had already started to restore an older one.

## Results

Both nodes of each valid pair halted at the same height (16,261,996 and
16,266,467), and the range took the same wall time on both (2,841 s).

| Metric | Pair | Go | celestia-rs | celestia-rs / Go |
| --- | --- | ---: | ---: | ---: |
| Time to verified tip (min) | `195144` p1 | 18.6 | 4.4 | 0.24 |
| Time to verified tip (min) | `232315` p1 | 16.9 | 17.7 | 1.05 |
| Peak cgroup memory over range (GiB) | `195144` p1 | 9.86 | 3.46 | 0.35 |
| Peak cgroup memory over range (GiB) | `232315` p1 | 11.20 | 3.72 | 0.33 |
| Peak process RSS over range (GiB) | `195144` p1 | 6.15 | 1.52 | 0.25 |
| Peak process RSS over range (GiB) | `232315` p1 | 7.35 | 1.60 | 0.22 |
| Node home after shutdown (GiB) | `195144` p1 | 4.9 | 1.8 | 0.36 |
| Node home after shutdown (GiB) | `232315` p1 | 5.0 | 1.9 | 0.38 |

The discarded `different_snapshots` pair is not part of the result, but its
range figures were in the same ranges: peak RSS 6.40 GiB against 1.51 GiB, peak
cgroup memory 10.24 GiB against 3.36 GiB, node home 5.0 GiB against 1.7 GiB.

## Snapshot discovery against snapshot restore

The time to the first local height splits into two parts. Discovery ends when
the data disk starts to grow by more than 200 MB. Restore ends at the first
local height. Both come from `polls.tsv` (5 s sampling).

| Run | Node | Discovery (s) | Restore (s) | First height (s) |
| --- | --- | ---: | ---: | ---: |
| `195144` p1 | Go | 67 | 921 | 988 |
| `195144` p1 | celestia-rs | 121 | 104 | 225 |
| `221446` p2 | Go | 33 | 889 | 922 |
| `221446` p2 | celestia-rs | 1,577 | 100 | 1,677 |
| `232315` p1 | Go | 39 | 893 | 932 |
| `232315` p1 | celestia-rs | 857 | 112 | 969 |

celestia-rs restores a snapshot about 8 times as fast as Go. But celestia-rs
needed 2, 14 and 26 minutes to find a peer that offered a snapshot at or above
its trust height, where Go needed about 1 minute. The discovery loop is the
same algorithm as in Go (`statesync/src/syncer.rs`, 15 s rounds). The
[2026-11-26 operator runs](operator-sync-2026-11-26.md) on `n2-standard-8` did
not show it: celestia-rs reached the tip in 307 s and 329 s there.

### Diagnosis

The discovery loop and the snapshot messages are correct: when a peer that
serves snapshots accepts the connection, celestia-rs logs `Discovered new
snapshot` in less than 1 s. The delay is the wait for such a peer.

- Open inbound slots are scarce. In a probe of the 20 peers from the reference
  `net_info`, 14 accepted TCP and then sent nothing (the listener of a full Go
  node does not accept), 2 refused TCP, 2 closed the connection and 2 started
  the handshake. One of those 2 served snapshots. It was open in some probes
  and full in others, and no silent peer answered in a 240 s wait. Both seeds are rejected on both sides (`peer is only
  using legacy propagation`), and of 293 address-book addresses that timed
  out, none answered a 5 s TCP probe.
- The order of arrival decided each pair. The node logs show that the Go node
  started 71 s to 73 s before celestia-rs in all three pairs and discovered
  its snapshot 12 s to 52 s after its start, before celestia-rs was running.
  In run `221446` the second Go node started 13 s after the first and
  discovered nothing in one hour. Go and celestia-rs dialed the same peers
  and logged the same errors.
- A late node then waited on the redial schedule. Both implementations redial
  a persistent peer 20 times (about every 6.5 s) and then back off
  exponentially (3 s, 9 s, 27 s, 81 s, 243 s, ...). After about 3.5 minutes a
  node tries each provider once in many minutes and misses the slots that
  open.

The fix: while celestia-rs knows no snapshot, each discovery round (15 s)
also redials the persistent peers that are not connected. In a 420 s local
mainnet run with these peers, the tries to the provider went from 25 (3 of
them after the first 3.5 minutes) to 56 (one about every 8 s to the end).
This does not create capacity: if no provider has an open slot, discovery
still waits.

To confirm on mainnet, start a cold node with `--log_level debug` and measure
the time from `starting state sync` to the first `Discovered new snapshot`.
During the wait, `Redialing persistent peers for snapshots redialed=N` must
appear every 15 s, also after the first 4 minutes. For a fair pair, start
both nodes at the same time: in these runs the Go node always started first.

## Limits

- Two valid pairs. All runs used live public peers, so the figures are
  observations and not guarantees.
- The time to the tip is not a stable comparison while the two nodes of a
  pair start at different times and compete for the same open peer slots (see
  Diagnosis).
- The nodes were freshly state-synced, so the disk figures show the footprint
  of a new node and not long-term growth.
- celestia-rs logs the configured halt as `CONSENSUS FAILURE: ... halt per
  configuration`. The halt itself was clean and the final heights matched.
