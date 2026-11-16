# Packet-forward memo limit oracle

Run `go run .` in this directory to regenerate `../fixtures/pfm-limits.json`.
The Rust integration test compares both receive rejection and retry eligibility
against these Go decisions, including malformed byte strings, numeric timeout
boundaries, Unicode keys, duplicate keys, and nested object/string memos.

The two `pfm_*.go` files are extracted from celestia-app commit
`9bb2bd1ac672cecaccd0b74bcefc79168f57158e` (PR #8629, still unmerged).
Adaptations: package `app` becomes `main`; the transfer memo length constant is
inlined as 32768; `rejectPFMForwardMemo` receives the already-decoded memo, so
the packet-data JSON decoding prefix is removed and `data.Memo` becomes
`memoText`. The memo checks and Go JSON decoder are unchanged.

This tests the memo helpers, not packet-data decoding, gas, refund balances,
or acknowledgement delivery. Rust middleware tests cover version gating and
timeout store/gas behavior. Re-pin this oracle if the upstream rules change
before merging the Rust change.
