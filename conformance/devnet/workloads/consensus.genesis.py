"""Genesis patch for the consensus workload: short gov periods, small deposits.

usage: consensus.genesis.py <genesis.json>
"""
import json
import sys

path = sys.argv[1]
g = json.load(open(path))
p = g["app_state"]["gov"]["params"]
p["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
p["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
p["max_deposit_period"] = "30s"
p["voting_period"] = "30s"
p["expedited_voting_period"] = "15s"
json.dump(g, open(path, "w"), indent=2)
