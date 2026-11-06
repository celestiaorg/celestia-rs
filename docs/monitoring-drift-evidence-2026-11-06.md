# Installed Mac monitoring comparison — 2026-11-06

Read-only collection on `mac-mini` at approximately 00:14 UTC. The
installed files were compared with repository `main` commit
`89807cde7970fbe824b7df89ead9509e8ff215cf`. No monitoring
service, installed file, validator, key or environment secret was changed.
This comparison does not establish the state after the collection time.

| File | Installed SHA-256 | Reviewed repository SHA-256 | Difference |
|---|---|---|---|
| `monitoring/bin/node-metrics.sh` | `87d673503edd3a584658e9b0d202393447c750de3487c8ac96a27aabdfc5bf98` | `ecd2247184e3c0a60947d0d038886824a79ef6940614d23fb39ee7a2d3dd97bc` | Installed collector derives jail status from tombstone/expiry of `jailed_until`; reviewed collector queries x/staking when a public operator address is configured, and omits the jailed metric on query failure |
| `monitoring/bin/check-apphash.sh` | `d64814641b26652b27c3d31dbe92f54769511732a922c3ea22fbe3df9f7341b7` | same | No difference against this main commit; [#298](https://github.com/celestiaorg/celestia-rs/issues/298) has a newer proposed receipt-producing checker |
| `monitoring/nodes.conf` | `5a31aa8272de712a55b14066c28d5eea4046c50c0b8f59d3f450d9429b54eb19` | `7402c9a1280fd11c17ff9098454a290de6e1961e9677078a4ab12649ec725f78` | Both installed lines have five fields and no public operator address; reviewed template includes the two public operator addresses |
| `monitoring/prometheus/rules.yml` | `4c30fa6974576f31d9502d465da487ec45c0b3412be9cdab41ba39d71cdc1246` | `ab6690f93a27f01eb50e93e7319245d2ad03f4fbb3d54e872928e6f3c24d0b4d` | Installed rules lack `ValidatorJailedUnknown` |

The installed file timestamps were Oct 19 for the app-hash checker, Oct 20
for `nodes.conf` and rules, and Oct 31 for the collector. A timestamp
does not establish a source commit; the hashes above identify the exact
files observed.

In a read-only probe using the service user's `HOME` and the installer's
launchd `PATH`, both configured public reference `/status` endpoints
returned HTTP 200. Their reported chain IDs were `mocha-5` and
`celestia`, respectively. This contrasts with the mainnet endpoint's
HTTP 403 during the [Nov 5 investigation](live-validator-evidence-2026-11-05.md);
it shows an endpoint response changed or varied, not why. The probe
recreated the service environment; it was not a new sample emitted by the
installed launchd collector.

The installed collector and rules still need replacement in an authorized
maintenance window. After installation, fresh samples must demonstrate
staking-derived jailed and unknown states, and the reference must be
checked from the actual collector job. Issue
[#297](https://github.com/celestiaorg/celestia-rs/issues/297) remains open.
