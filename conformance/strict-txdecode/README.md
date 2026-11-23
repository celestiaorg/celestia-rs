# Strict transaction decoder oracle

Run `go run .` here to regenerate `../fixtures/strict-txdecode.json`.
The module pins celestiaorg/cosmos-sdk tag `v0.53.8` (commit
`8bf1b2edb405d2d1072c4962b7d61bdb03559ded`), which holds the strict decoder
from SDK PR #214. celestia-app PR #8611 uses the same tag.
Keep the historical v9/v10 oracle modules pinned to their released versions.

The fixtures compare strict and legacy acceptance for wire-form and decimal
violations, exact Any URLs, large transactions, and recursion boundaries in
ModeInfo and multisig Any values. They also cover canonical single signatures
and nested multisignatures, signature counts at both levels, and the
multisig bit array: padding bits, `extra_bits_stored`, and one mode info and
one signature per selected key.
Rust checks acceptance and SDK error codes. The oracle registers SDK, crypto
and bank interfaces.

Re-pin and regenerate if the upstream rule changes before merging. These
fixtures do not resolve the separate release-version question in issue #385.
