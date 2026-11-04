# Transfer authorization boundaries

`transfer-authz-boundaries.script` is an exact signed v10.5.0 fixture; the
v9 file is byte-identical because both pinned apps use Celestia's ibc-go
v8.7.2 authorization implementation and the deterministic IBC scenario opens
`transfer/channel-0`. The coverage run waits 60 generated blocks for that
channel before sending one fixture transaction per block.

The first seven grants require the validation-order errors for empty
allocations, a duplicate channel, nil and zero spend limits, invalid port and
channel identifiers, and a duplicate receiver. The remaining transactions
exercise finite partial and exact consumption, failed-budget preservation,
receiver and memo policies, the MaxUint256 sentinel, removal of one allocation
while another remains, and rollback when authorization accepts a transfer but
the IBC handler rejects its missing channel.

`DELIVER_TRANSFER_AUTHZ_REQUIRE=1` fails unless all 28 signed outcomes match in
order, the driver established `channel-0`, exactly seven additional send
sequences committed, and the final missing-channel failures preserved the
last grant byte-for-byte. A normal signed `MsgExec` cannot reach
`TransferAuthorization.Accept`'s type-mismatch arm: grants are stored under
the inner `MsgTransfer` type URL and dispatch looks them up by the actual inner
message type before calling `Accept`.

These fixtures establish bounded differential and branch evidence. They do not
establish mainnet-shaped state, observation-window completion, or production
readiness.
