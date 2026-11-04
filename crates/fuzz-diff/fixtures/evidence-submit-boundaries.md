# MsgSubmitEvidence boundary fixtures

These exact signed fixtures cover the two reachable terminal outcomes of a
well-formed `MsgSubmitEvidence` in Celestia App's configured evidence keeper.
The v10 `evidence` scenario first resubmits an equivocation already stored by
the recorded block replay and requires `evidence/5`, then submits a new valid
equivocation and requires the recovered nil-router panic (`undefined/111222`). The v9 fixture
requires that same recovered panic for a new valid equivocation.

Celestia App does not install an evidence router at either supported version,
so a new valid submission cannot succeed. The expected-result files require
every exact transaction to be finalized in order; generated failures or an
omitted transaction cannot satisfy the regression.
