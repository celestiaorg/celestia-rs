# Open questions

Things that need the owner. Work continues around each item; the "Default"
column records what the code does until it is answered.

| # | Opened | Question | Default meanwhile |
|---|---|---|---|
| 1 | 2026-10-14 | **Partly resolved 2026-10-15.** HTTPS to public Celestia RPCs now works (mainnet: `rpc.celestia.pops.one`, `celestia-rpc.publicnode.com`; Mocha: `rpc.celestia-mocha.com` (archival from genesis), `celestia-testnet-rpc.polkachu.com`). **Raw P2P (tcp 26656 etc.) does not**: the environment's egress proxy accepts the CONNECT and then resets non-TLS traffic, so this container cannot join a network's P2P layer. A validator (Mocha or mainnet) therefore has to run on the owner's machine. | Follow testnets/mainnet over HTTPS RPC (fetch blocks + commits, verify with the light client, execute, compare app hashes); run P2P/consensus only on local devnets. |
| 2 | 2026-10-14 | **Answered 2026-10-15 (by querying the networks).** Mainnet runs celestia-app **v9.0.8** (`app_version` 9). Mocha is `mocha-5` (genesis 2026-08-18 at app v9) and runs **v10.4.0-mocha** since height **1,082,620** (2026-09-24). | Keep v10.4.0 as the primary target; add app-version-9 behavior (Go v9.0.8 parity) so the node can follow mainnet today, replay Mocha across its v9→v10 upgrade, and execute mainnet's v10 upgrade. |
| 3 | 2026-10-15 | **Owner action: fund the Mocha validator operator address `celestia160hc0ztelnza3lfz6xuyfwjy3mpts6ull7de9s`** (mocha-5, a few hundred TIA) so it can create a validator. Testnet only; the key is a local `test` keyring under `/home/user/mocha-validator` in the cloud container (not committed; regenerate if the container is reclaimed). Also: after network access was switched to Full, raw P2P from the current container is still reset by the egress proxy; the setting is expected to apply to a fresh container. | Run as an HTTPS-RPC follower until funded and until P2P works; then join P2P, create the validator, soak (M13). |
