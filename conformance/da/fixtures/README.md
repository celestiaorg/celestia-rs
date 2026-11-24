# v11 private-network controls

These blocks came from a fresh single-validator Go chain named `da-oracle`, running celestia-app v11.0.0 (`58be8ae90d00518800df5506b3a1e988f009c1c6`) and celestia-node v0.36.0 (`d5dfb21a0885622828e30e8e2daf50e5d1bb72b2`) in bridge mode. Both executables were built with `go build` from their pinned trees. The genesis SHA-256 was `3bb0fca8fdc874cc92507578f56a2eac64cba29bfb2cfca5b8657f09a7207373`. The app and bridge executable digests were respectively `a6646e915a8bfe7f25397524e11d2a3e3f3cc7dc420365846b2dbb35b69468ab` and `0e0870bec1b08ee244efa444193f8de12835dbb7504ad8360bc6a5ca0ec1443e`.

Block 9 has an empty square. Block 35 contains two blobs in the same Go transaction. Namespace ID `0101` holds 690 bytes of `0x61`, and `0103` holds 1150 bytes of `0x62`. Namespace `0102` falls in a row's namespace range and has an absence proof. The second blob spans two rows.

Capture another control with the script in the parent directory. Provide the consensus RPC URL, bridge RPC URL, read-token file, height, namespace IDs and destination. Only public block data and proofs are recorded.

Run the offline comparison with:

```sh
cargo test --locked -p app --no-default-features --features bridge --test bridge_control
```

## Mocha 5 release controls

The Mocha captures come from a separate private `da-oracle` chain. The app was
built from v11.0.0-mocha at `58be8ae90d00518800df5506b3a1e988f009c1c6` and the
bridge from v0.36.0-mocha at `af9897bb471959660a4179048656694f3013b2f5`, using
`go build` on their respective command packages. Height 2 is empty. Height 4
contains both paid namespaces with the same input bytes described above.

| SHA-256 input | Digest |
|---|---|
| Genesis | `2a455c809c8d94ee6a29e57de77291e125fe81d7e69bc67d3b5b92577309b2dc` |
| celestia-appd | `a6646e915a8bfe7f25397524e11d2a3e3f3cc7dc420365846b2dbb35b69468ab` |
| celestia bridge executable | `ee3c335dfba223c309e362e9c82a03fdfb2de48f01e3b3551b58e7ed99edbebe` |
