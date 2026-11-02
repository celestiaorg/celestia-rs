# Final candidate all-API soak

The signed candidate `f51ce1218e9c701108af0d038bf0dab1f2d2f3c1` passed a fresh
isolated mixed-node soak after the protobuf content-type and SDK error-metadata
fixes. Two Go validators, two Rust validators and a Rust full node ran for
**312 seconds and 112 height advances**. All **960 provider operations** completed:
624 query/subscription operations and 336 accepted broadcasts, across Comet URI,
JSON-RPC and WebSocket, core gRPC, Cosmos gRPC and REST. There were zero provider
errors, unknown admissions, consistency failures or stalls.

The contiguous archive observations cover heights 1–114 and 404 transactions.
All five nodes were checked through height 111; four were checked through 114.
The last Go node's consistency poll was three heights behind at cutoff; those
last three comparisons are not claimed. Maximum observed aggregate
RSS was 1,792 MiB; maximum aggregate node disk was 101 MiB. Indexed workload
transactions had no execution failure or loss. Recently accepted transactions at
cutoff remain separately accounted for; acceptance is not a claim of inclusion.

The oracle is the ledger-tagged standalone celestia-app v10.5.0 at
`6013e758d5dceb75827ad191c8d739d7c43cfa04`, core v0.42.3, built with Go 1.26.9.
Its fresh mainnet observation, exact recipe/dependencies and binary hashes are
in `provider-provenance.json`. That compiler differs from the original issue's
Go 1.26.6 baseline. Rust reports its own signed source commit and rustc 1.98.0.
The official Go 1.26.6 multiplexer is tested separately by the historical campaign.

The committed artifacts include report/events, contiguous hash observations,
resource samples, five public process logs, provenance, exact capture sources
and their manifest. Runtime homes, databases and private keys are excluded.
Normal WebSocket disconnect/shutdown diagnostics remain in raw logs; none
accompanied a failed provider operation. Capture-source verification uses the
archived files, so squash merging cannot make the source receipt unfetchable.

This is the memory profile: no restart/partition chaos, validator-set cycling,
long-profile authz/feegrant/Hyperlane/UTF-8 workload, TLS/proxies, external SDKs,
cryptographic proof checks, multi-page cursor traversal, every message family on
every transport, quota saturation, unbounded slow readers, or exact paired
response equality. These limits remain explicit in the raw report/provenance.
The module and persistence campaigns supply their separate matched-state cases;
this soak verifies concurrent progress and client behavior.

Reproduce with `conformance/devnet/soak.sh` using the recorded source/binaries,
a fresh `SOAK_MAINNET_OBSERVATION`, a new `SOAK_DIR`, `SOAK_PROVIDER_APIS=1`,
`SOAK_PROFILE=memory`, `SOAK_SECS=300`, `SOAK_MIN_HEIGHTS=80`, `SOAK_TX_RATE=10`,
`SOAK_LOAD_ACCOUNTS=48`, `SOAK_MEM_SAMPLE=10`, `SOAK_SEED=181`, and unused local
ports. Set `CELESTIA_RS`, `DEVNET_BIN` and isolated `CELESTIA_APP_HOME` explicitly.

```sh
python3 conformance/mainnet-rpc/provider-soak/v10.5.0/2026-11-02-f51ce12/verify.py
```
