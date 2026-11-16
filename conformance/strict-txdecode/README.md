# Strict transaction decoder oracle

Run `go run .` here to regenerate `../fixtures/strict-txdecode.json`.
The module pins celestiaorg/cosmos-sdk commit
`30a73d35dfaa41c662688acf819f864e15008b68` (SDK PR #186), the strict decoder
used by celestia-app PR #8611. Both upstream PRs are still unmerged.
Keep the historical v9/v10 oracle modules pinned to their released versions.

The fixtures compare strict and legacy acceptance for wire-form and decimal
violations, exact Any URLs, large transactions, and recursion boundaries in
ModeInfo and multisig Any values. Rust checks acceptance and SDK error codes;
error log wording is not compared. The oracle registers SDK, crypto and bank
interfaces, not the entire Celestia application registry.

Re-pin and regenerate if the upstream rule changes before merging. These
fixtures do not resolve the separate release-version question in issue #385.
