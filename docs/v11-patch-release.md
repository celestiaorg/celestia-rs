# v11 patch release and v12 consensus gates

The final [celestia-app v11.0.0](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0)
and the earlier tags `v11.0.0-corto` and `v11.0.0-mocha` resolve to
[`58be8ae90d00518800df5506b3a1e988f009c1c6`](https://github.com/celestiaorg/celestia-app/tree/58be8ae90d00518800df5506b3a1e988f009c1c6).
They are the v10 release line with the v11 emergency changes, rather than the
former v11 mainline rule set. This is the release assignment recorded by
[#385](https://github.com/celestiaorg/celestia-rs/issues/385#issuecomment-6930990079).

## Activation boundaries

`sdk::version::MAINLINE_RULES_FROM` is **12**. The SDK `is_v12` helper and
context-aware callers retain the executing-release override: an executing
binary can differ from the committed consensus version during an upgrade.

| Behavior | v11 patch | From v12 |
| --- | --- | --- |
| Seven unsupported ante messages (Go #8600) | Historical handler behavior | Rejected before the normal decorators |
| Blob params absent from module store (Go #8601) | Legacy subspace fallback | No legacy fallback |
| PFM error acknowledgements / gov failure reason (Go #8602) | Historical error text | Fixed acknowledgement / codespace and code |
| Signal version checks (Go #8599) | Header version, including historical zero-header behavior | Consensus app version; clear non-increasing reached upgrades |
| Strict transaction encoding (Rust #377, Go #8611 / SDK #186) | Historical decoder | Strict decoder, once its pending PR merges |
| Packet-forward memos (Rust #387, Go #8683) | Historical middleware | Pending adoption of the tagged Celestia middleware fork |
| In-process account sequence (Go #8443, #8449) | Read the signer’s check-state sequence | After the first commit, return the greater of the committed sequence and one past the highest pending sequence. Simulation accounts for pending transactions on its discarded branch. |

The reference tag's ante chain, blob keeper, signal keeper, encoding config,
and absence of the mainline PFM/gov wrappers establish these exclusions.
Propagation limits and PrepareProposal panic recovery have no state-machine
activation gate and are unaffected.

## Actual v11 behavior

The tag includes Go #8618 (`app/frozen/frozen.go`) and #8621
(`app/upgrades.go`). The **v11 executable** activates frozen-account rules when the committed
consensus version reaches 11. Go removed these hooks from the v12 executable.
Only the exact chain IDs `celestia`, `mocha-5`, and `corto-1` have lists.

The three address files and recovery addresses are copied from that exact
commit. Their byte hashes and unique counts (41,963 / 3 / 3) are pinned in SDK
tests. The mainnet recovery address retains the tagged source's value; no
untagged address correction is inferred. Changing a list or destination needs
a separately versioned consensus rule.

Bank restrictions run after debiting and before crediting, matching the SDK
send-restriction boundary, including multisend and account-to-module routes.
Listed senders may pay only their chain's recovery address or fee collector.
Transaction caches retain normal rollback semantics on a rejected send.

The ante decorator runs after NestedMsg and before FibreLegacyEscrow. It caps
listed signers' fees at 1,000,000 utia; rejects listed fee granters; allows only
send, undelegate, reward withdrawal, and setting the recovery withdraw address;
checks nested authz signers; and rejects Fibre promises from listed escrows.
Reward withdrawals inspect the pre-transaction withdraw address, including
inside authz. The Fibre validation query also rejects promises that cannot
settle, using the version read from consensus state.

The `v11` handler runs module migrations, then sets Corto's unbonding duration
to 600 seconds. It leaves other chains and existing unbonding queue entries
unchanged. It does not reset fibre accounts, rewrite grants or fee allowances,
change withdraw addresses, or add stores. Those operations are not in the tag.

The runtime recognizes a completed v11 upgrade or an explicit v11 genesis.
A signalled consensus version alone leaves a restored v10 node on v10.
The runtime authenticates a `v11`
`upgrade-info.json` against the committed next-height plan before recording
the executing release. A v10 execution remains v10 until activation/restart;
historical v9-to-v10 replay remains supported. New genesis defaults and the
existing v10 qualification target are unchanged.

## SDK inclusion-proof queries after the testnet tags

For executing releases 11 and later, the SDK's Tendermint `ABCIQuery`
service follows the v11.x backport at `1336489d9fed94210d06d4710404617efe0d8c57`.
It returns `PermissionDenied` for the two custom inclusion-proof routes.
Clients can use CometBFT's `tx`, `tx_search` and `prove_shares_v2` RPCs.
The Corto and Mocha binaries at `58be8ae9` predate this endpoint change.
Rust uses the executing release to select this service policy, including for
historical queries issued to a v11 node.

## Verification scope

`conformance/v11-frozen` generates decisions using the original tagged Go
decorator. Rust compares its error codes across versions, chain IDs, fees,
fee grants, authz messages and withdrawal destinations. This isolated oracle
does not measure store gas or prove whole-chain v11 app-hash equivalence.
Bank integration tests cover ordinary, module and multisend routes. Upgrade
tests cover Corto-only migration, unchanged version maps, authenticated
activation and persistence across restart. Existing v10 upgrade replay must
still halt where its recorded Go binary has no next-version handler.

These tests provide implementation evidence, not a new release qualification
or permission to alter a live validator. The project milestones still require
candidate-bound replay and observation evidence.
