"""Genesis patch for the vesting workload: turns the vp and vl genesis
accounts into a PeriodicVestingAccount (two periods) and a
PermanentLockedAccount.

usage: vesting.genesis.py <genesis.json>   (reads <home>/vesting.json)
"""
import json
import os
import sys

path = sys.argv[1]
home = os.path.dirname(os.path.dirname(path))
meta = json.load(open(os.path.join(home, "vesting.json")))
g = json.load(open(path))
accounts = g["app_state"]["auth"]["accounts"]


def base(acc):
    return {k: acc[k] for k in ("address", "pub_key", "account_number", "sequence")}


for i, acc in enumerate(accounts):
    if acc.get("address") == meta["vp"]:
        now = meta["now"]
        accounts[i] = {
            "@type": "/cosmos.vesting.v1beta1.PeriodicVestingAccount",
            "base_vesting_account": {
                "base_account": base(acc),
                "original_vesting": [{"denom": "utia", "amount": "600000000"}],
                "delegated_free": [],
                "delegated_vesting": [],
                "end_time": str(now + 120),
            },
            "start_time": str(now),
            "vesting_periods": [
                {"length": "40", "amount": [{"denom": "utia", "amount": "200000000"}]},
                {"length": "80", "amount": [{"denom": "utia", "amount": "400000000"}]},
            ],
        }
    elif acc.get("address") == meta["vl"]:
        accounts[i] = {
            "@type": "/cosmos.vesting.v1beta1.PermanentLockedAccount",
            "base_vesting_account": {
                "base_account": base(acc),
                "original_vesting": [{"denom": "utia", "amount": "700000000"}],
                "delegated_free": [],
                "delegated_vesting": [],
                "end_time": "0",
            },
        }
json.dump(g, open(path, "w"), indent=2)
