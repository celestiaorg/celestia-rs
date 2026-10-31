# Go oracle build identity

`build-bins.sh` builds the pinned celestia-app module with its standalone
Makefile recipe: the `ledger` build tag, SDK name/application/version/source
commit/build-tag metadata, and default v2 upgrade height. The source commit
comes from the module proxy's origin metadata, not the surrounding Rust checkout.
A plain `go build` omits a linked ledger descriptor and leaves `/abci_info` and
`/app/version` empty, so it cannot verify release registration or version fields.

This is a reproducible standalone source build, not an official release binary.
The multiplexer and embedded historical apps are excluded. Its compiler comes
from the existing pinned `GOTOOLCHAIN` policy and may differ from the original
mainnet binary. Capture evidence must name the actual compiler and tags, and
must not infer historical pre-v10 provider support from this build.

Each successful binary has three provenance files alongside its legacy
`.module` stamp:

- `.build.json`: module/version/checksum, source commit, actual compiler,
  build target, tags/linker flags and binary SHA-256.
- `.build-info.txt`: actual `go version -m` output, including dependencies.
- `.module`: the selected module/version for existing fixture tooling.

The builder checks recipe identity and binary content before reusing an
executable. Old module-only stamps, changed compilers/targets, and modified
binaries cause a rebuild. An app that reports the wrong software version is
rejected before replacing the output or writing a success stamp. A changed
source release always needs fresh isolated capture and a mainnet oracle check;
rebuilding does not update old fixture provenance.

The local cache regressions use an isolated fake Go compiler and perform no
network access:

```sh
python3 -m unittest discover -s conformance/devnet -p 'test_build_bins.py' -v
```
