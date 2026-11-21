# Open questions

These items need the owner's input. [Milestones](MILESTONES.md) tracks the
qualification work and its acceptance criteria.

**Scope clarified 2026-10-24:** target general recommendation for
non-archival RPC nodes, consensus nodes and validators against the current
Go target in ADR 0010 (v10.6.0).
Archival RPC, HSMs and remote signers are later goals. An external audit is
optional; a separate review agent is authorized for independent code review.
F1's measurable acceptance criteria remain in force (ADR 0008).

## Open

| # | Owner action | Current state |
|---|---|---|
| 10 | Authorize a new three-day mainnet observation window for the qualifying release candidate. | The November 20 audit found gaps in reference samples and failed comparisons at 45 mainnet heights. [Collector audit](docs/validator-observation-audit-2026-11-20.md). |
| 11 | File the upstream `gogoproto/jsonpb` oneof report. | The report is drafted and Rust's policy is recorded under F1.11. [Draft report](docs/upstream/gogoproto-jsonpb-oneof.md). |

## Recorded decisions and resolutions

| # | Opened | Decision or resolution | Current practice |
|---|---|---|---|
| 1 | 2026-10-14 | **Connectivity.** Public HTTPS RPC worked from the cloud environment. Raw P2P was blocked there. | Use `follow` over HTTPS in that environment and run public-network P2P on the Mac. Mocha verified state sync, block sync and consensus with Go peers. The mainnet canary also joined over P2P. [M13 and M14](MILESTONES.md). |
| 2 | 2026-10-14 | **Go versions.** Mainnet moved from celestia-app v9.0.8 to v10.4.0 at height 15,116,658 on 2026-10-20. Mocha moved to v10.4.0-mocha at height 1,082,620. | The current qualification target is v10.6.0. Keep v9.0.8 parity for historical replay. HTTPS replay matched the v9 → v10 upgrade. [M13 and F1.10](MILESTONES.md). |
| 3 | 2026-10-15 | **Mocha test funds.** The test received 1,000 TIA and returned 990 TIA the same day via a Rust CLI-signed MsgSend at height 1,698,327 (code 0). | About 10 TIA was retained for live testnet transactions. |
| 4 | 2026-10-15 | **Mocha validator settings.** The owner selected the moniker `celestia-rs validator`, 1 TIA self-delegation, 1 utia minimum self-delegation, commission 0.20 / 0.30 / 0.01, outbound-only networking and a source build. The validator bonded at height 1,720,896. | The macOS CI job runs on every push. [Mocha runbook](docs/operator-guide.md#11-running-a-mocha-validator-on-a-mac-m13). |
| 5 | 2026-10-15 | **Generate-only transactions.** A bech32 address passed to `--from` with `--generate-only` bypasses the keyring, matching Go v10.4.0. A key name still opens the keyring. | Use the operator address for offline transaction generation. Gas simulation with `--gas auto` still requires the public key. [Operator guide](docs/operator-guide.md). |
| 6 | 2026-10-15 | **WAL rotation.** The owner chose Go-compatible rotation on 2026-10-16. | The WAL rotates at 10 MB and retains about 1 GB of history. Go and Rust read the same WAL groups. An older head-only WAL opens without migration. |
| 7 | 2026-10-16 | **Mocha snapshots re-enabled.** An inline snapshot delayed Commit and caused three missed blocks. The snapshotter now runs in the background with its height pinned against pruning. | Background snapshots were enabled during the accepted M13 soak. The runbook uses an interval of 1500 after upgrading to a build with the fix. [Snapshot runbook](docs/operator-guide.md#1113-state-sync-snapshots-and-missed-blocks). |
| 8 | 2026-10-19 | **Canary and stake gates.** The owner chose C1 for a small-stake mainnet canary and F1 for more than canary stake. | The canary bonded early on 2026-10-19 with 1 TIA self-bond. C1 qualification and the F1 gate remain open. [Milestones](MILESTONES.md). |
| 9 | 2026-10-20 | **Canary operator key.** The owner accepted a hot `file` keyring for the mainnet canary so the watchdog can auto-unjail it. | The passphrase is stored in a mode-600 file on the Mac. The owner reaffirmed this decision for canary stake on November 5 and confirmed exclusive custody of the consensus key. [Owner attestation](docs/c1.5-owner-attestation-2026-11-05.md). |
