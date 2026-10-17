"""Genesis patch for the v9-upgrade-boundary workload: short gov periods and
small deposits, so a proposal submitted at app version 9 ends at 10.

usage: v9-upgrade-boundary.genesis.py <genesis.json>
"""
import json
import sys

path = sys.argv[1]
g = json.load(open(path))
p = g["app_state"]["gov"]["params"]
p["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
p["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
p["max_deposit_period"] = "60s"
p["voting_period"] = "90s"
p["expedited_voting_period"] = "30s"
json.dump(g, open(path, "w"), indent=2)
