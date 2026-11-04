# StakeAuthorization staking operations

These exact signed fixtures run after the recorded `staking-lifecycle` (v10)
and `v9-staking` scenarios. They create a deterministic second validator and
delegate enough stake to the recorded validator before exercising the three
staking authorization types that are not covered by the delegate fixture:
undelegate, redelegate and cancel-unbonding.

The companion expectation file requires every top-level transaction and Go
outcome in order. The sequence proves that an over-budget attempt preserves
the remaining allowance, an exact spend deletes the grant, redelegation policy
checks use the destination validator, and cancel-unbonding policy checks use
the source validator. The v10 and v9 fixtures differ only where their recorded
source validator and absolute unbonding creation height differ.
