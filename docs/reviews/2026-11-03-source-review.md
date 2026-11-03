# Partial independent source review, 2026-11-03

These source inspections were performed by the separate `review_signer` agent,
which did not write the port or modify code during these reviews. The outcome
for each listed scope was: **no additional concrete discrepancy established
within that scope**. F1.12 remains open. A source inspection and an inspected
test body do not establish a new passing runtime check.

The [manifest](2026-11-03-source-review.json) records review revisions, reference
commits and full-file hashes for the files containing the selected regions.
Those hashes identify source; they do not imply every line of a file was
reviewed. All 13 listed Rust files match their review-base bytes at recording
revision `d37f1a66216609d1de50400fff6ec61d75ed49eb`. This file comparison does
not transfer whole-candidate qualification from an older commit. Reference tags
identify compatibility source targets; deployed-mainnet binary identity remains
the separate F1.10 requirement.

## Feegrant

Rust review base: `5641e7d3837630e5899ba9d7e81ec372883d3cbd`.
Reference: feegrant v0.1.1, source commit
`7828a002440069cbc795c0fba618f4f61c1a2773`, and selected SDK v0.52.12 coin
helpers at `2dd23a60cf34c3ab90c8a4f15a7f35d9b2ac69b4`.
The feegrant commit is the peeled source commit, rather than its annotated tag
object. Both documented app targets use this feegrant module; this review adds
no independent historical-v9 normal-boundary replay.

Selected Rust regions in `x/feegrant/src/allowance.rs`: decoding and shape
checks, grant decoding, packing/canonical packing, expiration, validation,
acceptance, `safe_sub`/`raw_valid`, period reset and AllowedMsg gas/check/copy
behavior. In `lib.rs`, selected get/grant/update/revoke/use paths and tests
were inspected. Go comparisons used `basic_fee.go`, `periodic_fee.go`,
`filtered_fee.go`, selected `keeper/keeper.go` operations and selected
`types/coin.go` validation/subtraction helpers. `UseGrantedFees` was reread
while preparing this ledger.

The checks covered strict expiry versus equality; reset equality versus
strict lateness; zero period; period-before-absolute deduction; removal,
update and event ordering; filtered-message ordering/gas; and repacking an
inner allowance only after success without removal. A nil-versus-empty slice
hypothesis did not produce a valid transaction discrepancy: protobuf Any
packing collapses empty slices along the inspected path.

Test sources inspected included `nil_spend_limit_amount_panics_like_go`,
`grants_are_stored_as_go_repacks_them`,
`allowances_with_wide_field_keys_decode_as_gogoproto_does` and
`scenarioFeegrantRevokeGas`. The latter's 414 vectors concern expired
revocation gas, rather than general allowance boundaries. The later 225-vector
boundary addition has separate conformance evidence and PR review.

This inspection did not cover the full math/coin arithmetic implementation,
arbitrary custom allowance implementations, complete pruning/genesis/query
behavior or all malformed persisted state. Context tracing through Any/slice
packing and ICA relay code was not a whole-file audit of those dependencies.

## Staking

Rust review base: `d37f1a66216609d1de50400fff6ec61d75ed49eb`.
Primary reference: SDK v0.52.12 at
`2dd23a60cf34c3ab90c8a4f15a7f35d9b2ac69b4`. The selected complete
`x/staking/keeper/slash.go` file also matches SDK v0.52.8 at
`87a6e78fe762129fa05e9f8357c87cb2a085d2d4`; the other staking reference files
were not all independently compared against v0.52.8.

Reviewed regions were the three slash functions in `slash.rs`; selected
Unbond/Undelegate, UBD entry creation/completion, redelegation entry creation,
Begin/CompleteRedelegation and ValidateUnbondAmount bodies in `delegation.rs`;
selected delegation/UBD/RED lookup, iteration, removal and ID/index/type
helpers in `store.rs`; UBD AddEntry merge behavior in `types.rs`; and the
CancelUnbondingDelegation handler in `msgs.rs`. References were selected
bodies in Go keeper `slash.go`, `delegation.go`, `unbonding.go`,
`msg_server.go` and types `delegation.go`.

Checks included infraction-height/maturity/hold ordering; theoretical versus
actual slash amounts; destination unbondings before moved shares; pool burn
order and omission of zero-burn hooks; ID allocation before same-height entry
merge; indexes/hooks only for newly added UBD entries; completion index
deletion before payout; and secondary-index/partial-cancellation ordering.
The apparently stale unbonding-type mapping was also present in Go, and was
not reported as a new Rust conformance bug.

Selected test source inspected:

- `slash_burns_and_changes_exchange_rate`.
- `create_bond_delegate_undelegate_and_mature`: creation, partial cancellation
  and maturity payout. Same-height merge was source-compared, not demonstrated
  by this inspected test.
- `redelegation_from_unbonded_validator_completes_at_go_zero_time`.
- `redelegating_last_shares_of_unbonded_validator_fails`.

This inspection did not cover all validator power-index encodings, complete
end-block validator selection, full hooks/distribution internals, hold/release
APIs, genesis/query paths, every iterator or every staking test body.

## Consensus and local signing

Rust review base: `5a22054579a1a152cf36028b9e6dd0ba8e11908c`.
The Go comparison in this inspection was limited to selected
`consensus/state.go` proposal and vote signing paths in celestia-core v0.42.3,
commit `a5537d84a008d566abc503d2ea41e30f2301403d`. This is the documented
source target, not fresh evidence of a deployed mainnet core fork tag.

Selected `consensus/src/state.rs` regions: startup/catchup replay and
check-double-signing-risk references; peer/internal message handling;
proposal signing; stopping guard before precommit; commit block-save,
synchronous EndHeight and app-apply ordering; and vote signing. Selected
`privval/src/lib.rs` regions: LastSignState H/R/S checks/save, vote/proposal
signing, save-signed, forwarding and atomic-writer entry/setup. The latter
was not a new full atomic-write audit. Event/failure delivery references in
`consensus/src/service.rs` were inspected and reread during ledger preparation.

Checks covered vote WAL synchronization before signing; persisted guards
before releasing signatures; same-H/R/S canonical comparison and timestamp
reuse; regression refusal; synchronous own-message handling; and commit
ordering. Proposal WAL flush failure logs and continues, while vote signing
refuses on failure; those selected behaviors match the inspected Go paths.
This inspection did not freshly compare all Go `privval/file.go`, replay or
WAL filesystem implementation code. Prior signer JSON/atomic-write reviews
remain separate evidence.

Test-source inspection included persisted-before-return, failed vote/proposal
state write and directory failure assertions in `privval/src/tests.rs`, plus
selected `crash_restart_from_wal`, `crashes_at_many_points` and
`crash_between_precommit_and_commit_does_not_refuse` bodies/comments in
`consensus/tests/sim.rs`. WAL implementation/test files inventoried only by
symbols/test names are not included as fully reviewed source.

This inspection did not cover every reactor/network/gossip path, every WAL
rotation/repair branch, remote signers/HSMs, concurrent multiple-process signer
use, actual process/power-loss windows or native filesystem durability. No new
runtime crash check was executed by this review.
