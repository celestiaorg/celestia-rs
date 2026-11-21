# Strict transaction decoder oracle

Run `go run .` here to regenerate `../fixtures/strict-txdecode.json`.
The module pins celestiaorg/cosmos-sdk commit
`aeb605c12be19efae4cf614000c7dbb5fc15785a` (SDK PR #186), the strict decoder
used by celestia-app PR #8611. Both upstream PRs are still unmerged.
Keep the historical v9/v10 oracle modules pinned to their released versions.

The fixtures compare strict and legacy acceptance for wire-form and decimal
violations, exact Any URLs, large transactions, and recursion boundaries in
ModeInfo and multisig Any values. They also cover canonical single signatures
and nested multisignatures, plus signature counts at both levels.
Rust checks acceptance and SDK error codes. The oracle registers SDK, crypto
and bank interfaces.

Re-pin and regenerate if the upstream rule changes before merging. These
fixtures do not resolve the separate release-version question in issue #385.
