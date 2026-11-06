# Mainnet launch codesigning incident — 2026-11-05

Read-only follow-up collected on 2026-11-06 UTC for
[#299](https://github.com/celestiaorg/celestia-rs/issues/299).
The earlier [validator investigation](live-validator-evidence-2026-11-05.md)
recorded `OS_REASON_CODESIGNING` as launchd's last exit reason without
the cause. A narrow macOS unified-log window and the existing updater
log now identify the sequence below. Times are UTC.

| Time | Retained observation |
|---|---|
| 23:12:17 | Updater logged a deliberate mainnet deployment from `dbee4cd` to `b459a6e` at local height 15,625,801. |
| 23:12:17.433 | macOS AMFI reported the newly installed mainnet executable had “no CMS blob” and an “Unrecoverable CT signature issue”; the launch constraint failed. `xpcproxy` exited with `OS_REASON_CODESIGNING` after about 5 ms. This was a failed launch, not a Rust process panic. |
| 23:12:47.551 | launchd successfully spawned the mainnet executable after its throttle interval, PID 68823. |
| 23:14:12 | Updater logged 20 of 20 signatures across heights 15,625,824–15,625,843, then reported the mainnet node healthy and the deployment complete. |

At follow-up, `codesign --verify --verbose=2` reported the installed
mainnet executable valid on disk and satisfying its designated
requirement. The transient launch constraint failure occurred during
the deliberate updater restart. The log does not establish why the
first launch rejected that executable while the later launch
succeeded. It does establish that the stored last-exit reason must not
be counted as a separate Rust crash or an unresolved current
codesigning failure.

The updater currently atomically replaces the executable and
immediately calls `launchctl kickstart -k`; launchd has a 30-second
throttle interval. This incident introduced a roughly 30-second
failed-launch interval during a deployment. It interrupts a continuous
same-candidate observation window, even though the updater
subsequently recorded a healthy node. No update, restart, signing
operation or service configuration was performed for this follow-up.

The exact-candidate three-day at-tip record, every-height parity and
transaction-result evidence, and clean Mocha observation required by
C1.1 remain outstanding. Issue #299 and C1.1 remain open.
