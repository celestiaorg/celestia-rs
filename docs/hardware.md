# Target hardware

The reference machine for running a celestia-rs mainnet validator is a
**Mac Mini M4 (base): 10-core CPU, 16 GB unified memory, 512 GB SSD**,
macOS on `aarch64-apple-darwin`. Linux `x86_64`/`aarch64` are supported for
development and CI.

## Budgets

| Resource | Budget | Notes |
|---|---|---|
| Resident memory | ≤ 8 GB steady state, ≤ 12 GB peak | Leaves headroom for the OS and page cache. |
| Disk | ≤ 350 GB after 1 year | Validator-minimal pruning of blocks and app state. |
| Block processing | Finalize + commit a full-size block well inside the block time | Measured against the Go node in the post-mainnet benchmark phase. |

## Implications

- Default pruning keeps only recent state and blocks inside the evidence /
  unbonding window; archival mode is not a goal.
- Erasure coding, NMT hashing and signature verification must be
  multi-core-aware but must not assume more than 10 cores.
- No dependency may require x86-only intrinsics; everything must build for
  `aarch64-apple-darwin`.
