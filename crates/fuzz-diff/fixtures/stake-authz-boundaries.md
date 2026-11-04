# Signed StakeAuthorization boundaries

`stake-authz-boundaries.script` and `stake-authz-boundaries-v9.script` use the
deterministic funded accounts and respective genesis validators from the v10
and v9 `authz-feegrant` scenarios. Each of their 20 signed transactions
occupies one block after the harness funding block. Grants expire on
2100-01-01, outside the fixture interval.

The sequence exercises delegate authorization with:

- partial max-token consumption, an over-limit rejection, exact exhaustion,
  deletion, and a subsequent missing-grant rejection;
- allow lists with the real validator first, last, and absent;
- deny lists that match and do not match the real validator; and
- a nil max-token grant that remains usable after multiple executions.

The over-limit execution is followed by exact consumption of the previous
remainder, so a failed authorization that mutates its grant cannot pass. Every
successful `MsgExec` delegates to the scenario's real validator; authorization
acceptance followed by an unrelated staking failure cannot count as success.

The companion expected-results JSON requires every exact submitted transaction
to finalize in order with its pinned Go codespace, code, and diagnostic. The
ordinary differential still compares transaction results, gas, app hashes and
proposal behavior. The fixture does not cover undelegation, redelegation,
cancel-unbonding or expiration; those require separate state prerequisites.
