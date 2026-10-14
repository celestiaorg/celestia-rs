# Open questions

Things that need the owner. Work continues around each item; the "Default"
column records what the code does until it is answered.

| # | Opened | Question | Default meanwhile |
|---|---|---|---|
| 1 | 2026-10-14 | The cloud environment's network policy blocks public Celestia RPC/gRPC hosts (e.g. `rpc.celestia.pops.one`, `celestia-rpc.publicnode.com`), so new mainnet/Mocha fixtures cannot be fetched and the current mainnet app version cannot be confirmed. Please add the needed hosts under *Allowed domains* in the environment's network settings. | Use the local Go devnet as the differential oracle; keep the committed mainnet fixtures. |
| 2 | 2026-10-14 | Which app version is mainnet running right now (v9.x or v10.4.0)? | Target celestia-app **v10.4.0** (latest stable tag, 2026-10-01). |
