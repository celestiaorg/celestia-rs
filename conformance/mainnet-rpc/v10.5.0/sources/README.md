# Historical builder source

`build-bins.sh` is the exact builder bound by the 2026-10-31 v10.5.0 live
transaction and populated staking captures. Its SHA-256 is
`031981d6ca5911c603e96305472cabab9384a455ec257ba0a58beedfababab39`.
It was recovered unchanged from the parent of commit `e34841b`, which added
version-specific Go toolchain selection to the active builder.

The offline capture integrity checks verify this retained source against the
original receipt hash. The original metadata, binary provenance and captured
responses remain unchanged. All other source bindings still use their declared
repository paths. New builds use `conformance/devnet/build-bins.sh`; this file
exists only to retain historical capture provenance.
