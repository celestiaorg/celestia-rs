# v12 upgrade replay against celestia-app main

The official [v11.0.0 release](https://github.com/celestiaorg/celestia-app/tree/58be8ae90d00518800df5506b3a1e988f009c1c6) runs heights 1–5 on a deterministic test chain. A signed transaction signals v12 and tallies the single validator's vote. At height 5 the app commits the plan for height 6. [Upstream main at bb3a9e52d](https://github.com/celestiaorg/celestia-app/tree/bb3a9e52d09620323b35b8305922f764b2df6a3e) opens the same application database and executes heights 6–7. The fixture seeds legacy Fibre balances and withdrawals to exercise canonical signer merging and refunds.

Rust compares every app hash and the full available FinalizeBlock responses after the transition. It executes the same fixture continuously and after reopening before the upgrade block. This verifies the pinned main revision on the test chain. v12 is still in development, so parity must be rechecked against the release intended for each network.

| Height | App hash |
| --- | --- |
| 5 | `8de9facfbeb603a693401d2562d67a895630edec0d5aac11f197d23ed13c161c` |
| 6 | `968835b57c592846b6e135bc594f722a2f1a6334e5152989b50162cd521f8249` |
| 7 | `004eef9677d0e3c574458d07a66e1919e0f7a6e3630392185376fdeafaca154d` |

`abci.rec` SHA-256 is `10fb5460f29214b155e1766dfb7c6e30f75ccd790f870b6126e52c4b9d06ff74`.

To reproduce, place `capture_test.go` in the `app` directory of each pinned Go checkout. Set `CELESTIA_V12_CAPTURE` to the same empty absolute directory for both commands. In the v11 checkout, then in the main checkout, run `go test ./app -run '^TestCaptureV12Boundary$' -count=1`. Compare the output recording with the committed fixture. Each checkout's `go.mod` pins its toolchain. Run the Rust comparison with `cargo test --locked -p devnet-conformance --test v12_upgrade`.
