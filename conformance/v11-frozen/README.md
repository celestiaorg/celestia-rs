# v11 patch-release frozen-account oracle

Run `go run .` here to regenerate `../fixtures/v11-frozen.json`.
The module pins celestia-app `v11.0.0-mocha`; `v11.0.0-corto` points to the
same commit, `58be8ae90d00518800df5506b3a1e988f009c1c6`.

These fixtures call the original `frozen.Rules.AnteHandle` with the embedded
network lists. They cover activation at 10/11/12, chain isolation, allowed and
disallowed messages, fee caps/denominations, listed fee granters, withdrawal
destinations, nested authz, and malformed Fibre promises. The withdrawal keeper
is a deterministic stub; this oracle compares decisions and error codes, not
store gas or the complete signature/ante chain. Rust bank and upgrade tests
cover the normal send boundaries and the chain-specific migration separately.

The lists copied into `sdk/src/frozen` retain their original bytes. Hashes,
counts and recovery addresses are pinned in SDK tests. These are consensus
inputs; updating a list or destination requires a separate versioned rule.
