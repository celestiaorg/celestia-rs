# Finite follower final-header regression — 2026-11-12

`follow --stop-at-height H` previously returned success before comparing
execution at H with header H+1. That skipped the final app hash and transaction
results hash checks, even though the next header had already been fetched.
The fix compares and authenticates that header before reporting completion,
without executing block H+1. The follower may fetch through H+2 to obtain
H+1's canonical commit. Light-only mode retains its header-only behavior.

## Controlled reproduction

The test used three independent copy-on-write clones of the stopped,
isolated mainnet replay database retained from the November 6 investigation.
Only `application.rocksdb`, `blockstore.rocksdb` and `state.rocksdb` were
copied. No live validator database, signing key, keyring or service was used
or changed. The common checkpoint was height **15,531,673**, app hash
`3C15AC889DBB6B6EB699F7D7427367F83042CBEC69FC9F9BFC3EC36212DE2A5C`.

Each run executed exactly block **15,531,674** (three transactions). A
loopback-only proxy forwarded public archive RPC reads and changed header
**15,531,675**'s `app_hash` from
`11ACDC3D3DC2EF79FA95C2EED038E2217A398B4E62EEFCB0206B6F2138D646EB`
to `01ACDC3D3DC2EF79FA95C2EED038E2217A398B4E62EEFCB0206B6F2138D646EB`.
The fault affected the reference response, not the executed transaction data.

| Run | Binary source | Result |
|---|---|---|
| Old code, corrupted final header | `46acbcb516c3dab26cb5706c908dfe0207f27349` | Exit 0 after one block; the injected mismatch was missed |
| Fixed code, same fault and initial checkpoint | `ba28bf9da7d30fedad9bf573c977616a3410c301` | Exit 1 with the exact final `app_hash` mismatch and diagnostics |
| Fixed code, honest Pops and Polkachu archive references | Same fixed source | Exit 0 after one block; `verified_header_height=15531675` |

The fixed binary SHA-256 was
`5fee1b8084060ac31c378da4c82ee86bc484dabd1ec415dc0cc59c30d163a53b`;
the old binary was
`fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192`.
All source/input hashes stayed unchanged during their checks. The existing
three focused follower unit tests, formatting and diff checks also passed.

The [manifest](evidence/follow-stop-2026-11-12/manifest.json) binds the source
wrappers, logs, proxy source, injected field values and diagnostic summary.
The old-fault wrapper's `passed` status records the reproduced faulty command's
zero exit; it is **not** a parity pass. The fixed-fault wrapper's `failed`
status is the expected rejection. Its mismatch is deliberately injected,
not an unresolved discrepancy against the real network.

This is a bounded regression of follower completion semantics. The physical
checkpoint remains local and is not embedded in the repository; it was not
recreated from a fresh provider archive for this test. This test does not
establish historical-era replay coverage, live validator qualification,
cross-architecture equality or completion of #298/#299/#302. The earlier
isolated replay's independently retained per-height result receipts remain
separate evidence; a bare finite-follower success alone did not prove its
last height before this fix.
