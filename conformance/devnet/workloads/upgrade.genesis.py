"""Genesis patch for the upgrade workload: short gov periods, small deposits
and no circuit-breaker entries so MsgSoftwareUpgrade / MsgCancelUpgrade can
execute.

usage: upgrade.genesis.py <genesis.json>
"""
import json
import sys

path = sys.argv[1]
g = json.load(open(path))
p = g["app_state"]["gov"]["params"]
p["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
p["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
p["max_deposit_period"] = "60s"
p["voting_period"] = "60s"
p["expedited_voting_period"] = "30s"
g["app_state"]["circuit"]["disabled_type_urls"] = []
json.dump(g, open(path, "w"), indent=2)
