# Tagged v11 upgrade replay

The original [`upgrade`](..) recording has 99 commits on the local `test` chain. At height 99, celestia-app v10.4.0 sets the consensus app version to 11. Its next `FinalizeBlock` halts and writes `data/upgrade-info.json` for plan `v11` at height 100.

The capture driver replays the original ABCI requests through that halt. It verifies all 99 app hashes and opens separate copies of the committed v10 home with the official Mocha and Corto v11 binaries. Each binary receives the original height-100 `PrepareProposal`, `ProcessProposal` and `FinalizeBlock` bytes, then a `Commit` request. The two recordings are byte-identical, and both binaries report the same app hash on reopen.

| Height | App hash (hex) |
| --- | --- |
| 99 | `bd57f86cd1f1cc0199920144141f57da7269c4a9bf306c168eef5af5f3b44d0d` |
| 100 | `cda9afcaaeddcc1f7697e49c6613e1a4da7059cf3f0c5abc3d73ac45dc742e4f` |

The replay covers the offline application restart on chain ID `test` with an empty height-100 proposal. Corto-specific staking parameters and the CometBFT handshake are outside the fixture’s scope.

## Provenance

| Tag | Source commit | Linux x86-64 standalone archive SHA-256 | Extracted `celestia-appd` SHA-256 |
| --- | --- | --- | --- |
| [v10.4.0](https://github.com/celestiaorg/celestia-app/releases/tag/v10.4.0) | `5187d2fb5eb8bc4b534c74724882943c54253ae9` | `16add55d6f401b6f29d86cdbdb3948b7dc4015736b573d46ea66f0531195a1cd` | `66ee7b2b72c6406154113d9a3a15a45838cfaec16865b2df7aa479cf218cf902` |
| [v11.0.0-mocha](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0-mocha) | `58be8ae90d00518800df5506b3a1e988f009c1c6` | `addc971635bc91bb666d8a05f90d5b7db7983f993ea5b0540eb93edb7399676d` | `8ee28e467e6a53d4d36216825a567d97a9208fb20a1613ac77eb12ec7db4e689` |
| [v11.0.0-corto](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0-corto) | `58be8ae90d00518800df5506b3a1e988f009c1c6` | `7804c1f1546e7786ca8402b4201ef377cea86a33a2800d1d1b403d00024394ec` | `bd4d4ced675de95e4c98418204d2fd6b4832333d1c23aefe1812f2c1e23a9588` |

`celestia-appd version --long` reports Go 1.26.6 for v10.4.0 and Go 1.26.10 for both v11 artifacts.

| Fixture file | SHA-256 |
| --- | --- |
| `upgrade/genesis.json` | `55cf51151c75725e067fdd2cce8b6e091e36d6dff2b7abbac5cd60d72e18bab3` |
| `upgrade/abci.rec` | `7d3de22df1c1f1a0cad3d7895fc3f950fabcacc76ef1b80ee6ecb63cd4631b20` |
| `upgrade/v11-tagged-upgrade/abci.rec` | `a3233b9e71ff04868828f8afeae3e1f26223f7cbcc9f1a02294d5641d4c4f6c3` |

## Reproduce

Run from the repository root on Linux x86-64 with Python 3.11 or later. The replay uses loopback ABCI sockets.

```bash
set -euo pipefail
artifact_dir="$PWD/target/v11-upgrade-artifacts"
archive=celestia-app-standalone_Linux_x86_64.tar.gz
for tag in v10.4.0 v11.0.0-mocha v11.0.0-corto; do
    mkdir -p "$artifact_dir/$tag"
    gh release download "$tag" -R celestiaorg/celestia-app \
        -D "$artifact_dir/$tag" -p "$archive" -p checksums.txt
    (
        cd "$artifact_dir/$tag"
        awk -v name="$archive" '$2 == name' checksums.txt | sha256sum --check --strict
        tar -xzf "$archive" celestia-appd
    )
done
python3 conformance/devnet/probe-v11-upgrade.py \
    --artifacts "$artifact_dir" --out target/v11-upgrade-capture
cmp target/v11-upgrade-capture/abci.rec conformance/fixtures/devnet/upgrade/v11-tagged-upgrade/abci.rec
cargo test --locked -p devnet-conformance --test v11_tagged_upgrade
```
