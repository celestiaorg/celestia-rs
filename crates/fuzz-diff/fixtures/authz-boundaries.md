# Signed authorization boundaries

`authz-boundaries.script` uses the deterministic funded accounts printed by
`fuzz-diff accounts`. Each of its 27 transactions occupies a separate block;
`MsgExec` can contain multiple inner messages. The run requests 28 generated
blocks, including the harness funding block before the script starts. Grants expire on 2100-01-01,
well after the recorded fixture times. These are local harness accounts.

The pinned regressions run the script on `authz-feegrant` and
`v9-authz-feegrant`, comparing the full signed transaction/block path against
celestia-app v10.5.0 and v9.0.8 respectively. The companion expected-results
JSON is an additional reachability contract, not an alternative to the
existing tx-result, gas, proposal and app-hash comparisons.

The sequences exercise:

- SendAuthorization budget exhaustion, including a subsequent missing grant.
- First/last allow-list entries, a missing recipient, and spend subtraction
  before recipient filtering. Rejected accepts must preserve the remaining
  budget, which is then consumed exactly.
- Two inner sends using the same grant in one MsgExec, followed by exact
  consumption and deletion of the remainder.
- A successful inner send followed by an independently authorized delegate to
  a valid but nonexistent validator. The staking handler fails; spending the
  full original send budget afterward requires actual transaction rollback.
- Replacing a partially used grant with the same expiration and a new budget,
  then consuming more than the old remainder and exactly exhausting the new
  grant.

The delegate uses GenericAuthorization to reach the staking handler; this
fixture does not exercise StakeAuthorization policy or expiration boundaries.
App hashes retain the existing differential check on state changes. There is
no separate assertion of the post-failure bank balance or event rollback.

`DELIVER_SCRIPT_REQUIRE` names a nonempty JSON array with one object per script
transaction, in order: `type_url`, `codespace`, `code`, and optional
`log_contains`. Unknown fields and inconsistent success codespaces are rejected.
A required script must sign every entry and finalize the exact submitted bytes
in order; dropped/replaced/reordered transactions, too few blocks, incorrect
results and incorrect failure logs fail even if both implementations agree.
The Go outcome is checked against the contract after the ordinary differential
comparison. The two runs are part of `nightly.sh regressions`.
