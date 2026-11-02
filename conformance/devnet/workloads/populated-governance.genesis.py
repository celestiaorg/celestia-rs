"""Keep populated governance proposals active for the focused #188 capture."""
import json
import sys


path = sys.argv[1]
genesis = json.load(open(path))
params = genesis["app_state"]["gov"]["params"]
params["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
params["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
params["max_deposit_period"] = "600s"
params["voting_period"] = "600s"
params["expedited_voting_period"] = "300s"
json.dump(genesis, open(path, "w"), indent=2)
