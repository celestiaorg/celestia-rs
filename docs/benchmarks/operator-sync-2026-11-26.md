# Operator sync and equal-work DA runs, 2026-11-26

These runs validate two changes made after the [2026-11-25 operator
runs](operator-sync-2026-11-25.md): automatic state sync no longer accepts a
snapshot below its trust height, and the DA comparison now requires both
implementations to retain and serve the same heights.

The result is two valid cold mainnet pairs. celestia-rs reached a verified tip
4.9 times and 4.1 times as fast as Go. Over the subsequent matched 600-block DA
window, celestia-rs used 64.8% and 65.6% less peak sampled cgroup memory than
the Go stack. This replaces the earlier 6.0 GB against 27.7 GB observation,
where the Go bridge was filling about 140,000 older heights and the workloads
were not comparable.

## Implementations and environment

- celestia-rs commit `258da3a8887a2df2644b59588e1a36620f24e2b6`,
  executable SHA-256 `04bd2e73ff31926a6c87119808369df31628c3fdad66f8333e40b8893624dc7d`.
- celestia-app `v11.0.0`, commit
  `58be8ae90d00518800df5506b3a1e988f009c1c6`, executable SHA-256
  `003b1d9936e0cdb6609b660ac6bfd544ea7ada4d4caf65b787fd6b18359ce452`.
- celestia-node `v0.36.0`, commit
  `d5dfb21a0885622828e30e8e2daf50e5d1bb72b2`, on the Go side. Its executable
  SHA-256 was `32c3a78f…ba9977` in pair 1 and `248d09e6…334ee` in pair 2.
- Four cold `n2-standard-8` VMs, each with an empty 200 GB `pd-balanced` disk,
  in `us-central1-a`. The node configuration and network limit are the same as
  the [operator comparison protocol](operator-sync.md).

The [provisioning run](https://github.com/celestiaorg/celestia-rs/actions/runs/45773708363)
started all four VMs at 07:14 UTC. The
[final collection](https://github.com/celestiaorg/celestia-rs/actions/runs/45790059703)
reported `done-ok` for every VM and `valid` for both pairs. The artifact is
`operator-bench-bridge-45773708363`, artifact ID `15101039244`, SHA-256
`0782ab7b7f5382f058ed9775c8e2d3ce1e3986787f8d2df356cc62243400ff15`.

## State sync to verified mainnet tip

All four nodes used trust height 16,240,000 and restored snapshot 16,242,000.
After each node met the tip rule, its block ID was checked against an
independent mainnet RPC at that node's verification height. Thus both
comparisons pass the protocol's same-snapshot and block-ID gates. The common
height row below is a crossing-time comparison, not the verification height.

| Measure | Go, pair 1 | celestia-rs, pair 1 | Go, pair 2 | celestia-rs, pair 2 |
| --- | ---: | ---: | ---: | ---: |
| First post-restore height | 1,370.7 s | 199.6 s | 1,266.9 s | 199.5 s |
| First verified tip | 1,492.8 s | 306.5 s | 1,361.2 s | 328.7 s |
| Time-to-tip speedup | 1.0x | **4.87x** | 1.0x | **4.14x** |
| First poll at or above height 16,243,400 | 1,448.4 s | 306.5 s | 1,339.0 s | 306.5 s |
| Peak cgroup memory before DA | 12.34 GB | 5.96 GB | 12.30 GB | 5.99 GB |
| RSS at verified tip | 6.56 GiB | 1.38 GiB | 6.32 GiB | 1.35 GiB |

These are the first valid mainnet pairs after the old-snapshot fix. Automatic
discovery now rejects an offered snapshot below the configured trust height
before it consumes the peer's snapshot quota or starts light-client
verification. An operator can still restore historical state by deliberately
choosing a trust anchor at or before that snapshot. The two successful pairs do
not prove a failure rate. A regression test directly covers the stale case that
previously accepted height 11,643,500 with trust height 16,214,000 and spent 59 minutes
trying to verify a candidate separated from the trust anchor by 4,570,500
heights.

## Equal-work DA bridge memory

After consensus sync, celestia-rs restarted with its integrated bridge. The Go
VM started celestia-node beside celestia-app in the same cgroup slice. Both
sides retained the same tail, 16,242,001, and entered and left the measured
window at exactly 16,245,002 and 16,245,602. Go's sampled chain was contiguous
through both boundaries.

The client made the same 32 `share.GetShare(0,0)` reads on each side: the tail,
the start, and every 20 heights through the end. At each height all four
services returned identical canonical 512-byte shares. All four receipts have
SHA-256 `b63e90e98f74e6ebe790d385d81226e12d8c781696c0eb635e8b455d2bea95e0`.

| Measure over the 600-block window | Go, pair 1 | celestia-rs, pair 1 | Go, pair 2 | celestia-rs, pair 2 |
| --- | ---: | ---: | ---: | ---: |
| Samples / elapsed | 338 / 1,704.55 s | 339 / 1,704.33 s | 339 / 1,704.57 s | 339 / 1,704.94 s |
| Peak sampled cgroup memory | 11.54 GB | **4.06 GB** | 11.88 GB | **4.09 GB** |
| Median cgroup memory | 11.32 GB | **3.65 GB** | 11.59 GB | **3.55 GB** |
| Peak process-tree RSS | 6.14 GiB | **1.52 GiB** | 6.32 GiB | **1.51 GiB** |
| Median process-tree RSS | 5.93 GiB | **1.45 GiB** | 6.11 GiB | **1.45 GiB** |
| CPU time | 768.3 s | **512.7 s** | 592.4 s | **492.4 s** |

On the all-services cgroup measure, celestia-rs used 35.2% and 34.4% of Go's
peak sampled memory: 64.8% and 65.6% less. Its peak process-tree RSS was 75.3%
and 76.1% lower. CPU time was 33.3% and 16.9% lower. Samples were taken about
every five seconds. These are two observations under live public-peer traffic,
not confidence intervals or a universal capacity claim.

The comparison includes the whole operator stack on each VM. The Go figures
include celestia-app and celestia-node; the Rust figures include the integrated
consensus and bridge process. The fixed range and matching read digest
establish equivalent retention and client-visible work; uncontrolled peer
traffic can still differ. Cgroup memory includes charged file cache beyond
process mappings. Process-tree RSS includes resident file-backed mappings and
can double-count shared pages between processes.

## Failed precursor and cost

An earlier [provisioning attempt](https://github.com/celestiaorg/celestia-rs/actions/runs/45764951529)
did not produce a valid pair. GCE invoked the startup program concurrently on
some VMs. Both pair 1 nodes reached verified tip, but their earliest retained
heights differed by 1,500; duplicate setup then left them `done-failed`. Pair 2
Rust also ended `done-failed`, while pair 2 Go was still syncing at deletion.
No DA result from that attempt is used here. The driver now claims startup with
an atomic directory creation, with a regression test that races 20 claimants.

The failed attempt ran for about 46 minutes and cost an estimated $1.79 at the
workflow's conservative all-in rate. The valid replacement ran for 2 hours 2
minutes before early deletion, estimated at $4.77. Total estimated cloud spend
was $6.56, below the authorized $10 ceiling. The
[deletion run](https://github.com/celestiaorg/celestia-rs/actions/runs/45790159310)
was dispatched as soon as collection completed. A subsequent
[existence check](https://github.com/celestiaorg/celestia-rs/actions/runs/45790410294)
received `404` for all four VM names. Their boot and data disks were configured
to auto-delete with the VMs.
