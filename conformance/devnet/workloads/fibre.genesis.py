"""Genesis patch for the fibre workload.

Short x/fibre windows so withdrawals mature and promises time out within
the devnet run (InitGenesis does not validate params, so values below the
MsgUpdateFibreParams bounds are accepted): 60 s withdrawal delay, 30 s
payment promise timeout. Short gov periods and a small deposit for the
MsgUpdateFibreParams proposal.

usage: fibre.genesis.py <genesis.json>
"""
import json
import sys

path = sys.argv[1]
g = json.load(open(path))
f = g["app_state"]["fibre"]["params"]
f["withdrawal_delay"] = "60s"
f["payment_promise_timeout"] = "30s"
p = g["app_state"]["gov"]["params"]
p["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
p["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
p["max_deposit_period"] = "30s"
p["voting_period"] = "30s"
p["expedited_voting_period"] = "15s"
json.dump(g, open(path, "w"), indent=2)
