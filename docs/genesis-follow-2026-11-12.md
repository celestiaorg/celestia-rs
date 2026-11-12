# Bounded Mocha genesis follower check — 2026-11-12

A clean build of `ecec011ca435f9a7c9f8c233506d21731d114492` initialized an
isolated non-signing home from the public Mocha-5 genesis and executed heights
**1–100**, then authenticated and compared **header 101** before returning
success. The headers report app version 9. The interval contained **zero
transactions**, so this establishes startup, state/header checks and finite-run
completion, without transaction-result coverage.

The [manifest](evidence/genesis-follow-2026-11-12/manifest.json) binds the binary,
genesis and retained artifacts. The [build](evidence/genesis-follow-2026-11-12/build/evidence.json)
and [replay](evidence/genesis-follow-2026-11-12/replay/evidence.json) wrappers both
passed with clean, unchanged source. The genesis SHA-256 matches the repository's
Mocha network pin and was checked before and after replay. RPCs were the public
Mocha endpoint and Polkachu testnet endpoint; an explicit 10,000-hour trusting
period covered historical headers. The only configuration copied into the
isolated home was the public genesis. Live validator services and signing files
were left unchanged.

Offline RPC tests separately cover successful startup/resume, a forged first
commit rejected before execution, a wrong genesis chain, the app's existing
initial-height-1 restriction, and snapshot-only flags supplied without a snapshot.

This transcript does not retain an individual receipt for each height, complete
Mocha's v9 era or cross its upgrade. The source is the new follower, whereas the
installed validators remain on `46acbcb`. Neither
[#302](https://github.com/celestiaorg/celestia-rs/issues/302) nor F1.8 is closed by
this bounded check.
