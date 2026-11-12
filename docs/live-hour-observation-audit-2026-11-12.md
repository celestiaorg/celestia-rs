# One-hour validator observation audit — 2026-11-12

The **02:00–03:00 UTC** window does **not** establish clean observation or
qualification on either chain. Both validators retained candidate `46acbcb`
and the measured executable SHA-256
`fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192`.
The offline auditor reports the following bounded results:

| Measurement | Mocha-5 | Mainnet (`celestia`) |
|---|---:|---:|
| Inclusive executed heights | 2,538,784–2,540,038 | 15,811,303–15,812,568 |
| Heights from independent execution logs | 1,255 | 1,266 |
| Strict app-hash / tx-result covered heights | 1,252 | 1,266 |
| Valid status samples | 694 | 695 |
| Failed status samples | 1 | 2 |
| Valid-sample gaps above 20 seconds | 0 | 1 (21.772241 s) |
| Reference commits reporting this validator signed | 1,245 | 1,264 |
| Reference commits reporting this validator missed | 10 | 2 |

The execution ranges came from the actual validator log lines timestamped
within this hour, independently of receipt coverage. Every height in both
ranges has a later matching comparison receipt, but the strict auditor
preserves failed attempts: Mocha heights **2,538,883**, **2,539,031** and
**2,540,007** each had reference TLS timeouts before successful retries and
remain rejected in strict coverage. Those unavailable rows have no reference
chain identity, so the existing auditor counts them as `identity_rejected`;
this does not mean the local executable identity changed. No mismatching app
hash or transaction-result outcome was observed in these retained rows.
Transaction-result comparison covers code, data, gas wanted and gas used.

Mocha's failed status sample was at 02:12:16.194211Z. Mainnet's failures were
at 02:12:11.143194Z and 02:12:19.429570Z, with a valid-sample gap from
02:11:58.331902Z through 02:12:20.104143Z. Successful height retries do not
remove those observation failures. There were no recorded collector terminal
or executable-change events. Reference participation counts are RPC assertions;
the collector does not independently verify those signatures cryptographically.
The missed signatures occurred at scattered heights and are not attributed to
the status-query timeouts or any particular workload by this evidence.

Retained [mainnet](evidence/live-hour-2026-11-12/mainnet-audit.json) and
[Mocha](evidence/live-hour-2026-11-12/mocha-audit.json) reports both set
`qualification_established=false` and `sample_requirements_met=false`.
The [manifest](evidence/live-hour-2026-11-12/manifest.json) binds the compressed
frozen receipts and independent execution-log slices. These include earlier
and later rows needed for sample-edge and identity checks; the requested
window and height ranges remain explicit. Original decompressed hashes are
in [input-digests.json](evidence/live-hour-2026-11-12/input-digests.json).

The retained resource context contains 30-second Prometheus samples for this
hour: one-minute load was 1.76–5.47, swap used was approximately 86–120 MB,
and sampled `catching_up` stayed zero on both networks. These samples establish
bounded context, not a cause for misses or proof of continuous uptime. No
validator WARN/ERROR log lines were found in 02:11–02:13 UTC; that bounded
search does not exclude failures elsewhere or at other log levels.

To reproduce, decompress each `.gz` file to its original name in a temporary
directory and run `scripts/quality/observation_audit.py` with the manifest's
window, candidate/binary identities and the respective inclusive height range.
Both invocations exit 1 for these retained gaps; that is the expected result.
Neither [#299](https://github.com/celestiaorg/celestia-rs/issues/299) nor the
three-day gate is closed. Live services, signing files and the observation
criteria were left unchanged.
