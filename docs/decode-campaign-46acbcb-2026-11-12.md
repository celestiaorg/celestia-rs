# Exact-candidate decoding campaigns — 2026-11-12

Candidate **46acbcb516c3dab26cb5706c908dfe0207f27349** completed two bounded
release-harness campaigns on the Mac Mini, against pinned Go app v10.6.0 and
v9.0.8. Each selected operation (`tx`, `blobtx`, `iw`, `fibre`) exceeded
100 million inputs in each version, with zero differential mismatches.

| Operation | v10 inputs | v10 Go accepts | v9 inputs | v9 Go accepts |
|---|---:|---:|---:|---:|
| `tx` | 100,501,756 | 8,827,015 | 103,473,674 | 10,360,453 |
| `blobtx` | 100,494,277 | 743,788 | 103,465,113 | 2,617,496 |
| `iw` | 100,494,977 | 2,307,045 | 103,464,425 | 2,374,994 |
| `fibre` | 100,509,090 | 104,216 | 103,474,264 | 121,904 |
| Total | 402,000,100 | 11,982,064 | 413,877,476 | 15,474,847 |

Both invocations used seed `2026111202`, an iteration ceiling of one billion
and a 7,200-second budget. The harness reported 7,200 seconds for v10 and
7,201 for v9. Both clean-source qualification wrappers passed, preserving the
same source tree and input executable hashes through completion. The harness
returns failure if any mismatch family is found; neither run emitted a
mismatch or created its findings directory.

[Retained evidence](evidence/decode-campaign-46acbcb-2026-11-12/summary.json)
includes per-operation counters, both complete transcripts, qualification
manifests, oracle build logs and `go version -m` output. The release Rust
harness SHA-256 is
`0ba760fd0d6b857b9b06beb0aedd664f7943c59779e02e7d9aa3084b7a919f07`.
The v10 oracle was built with Go 1.26.10 and v9 with Go 1.26.9; the wrapper's
ambient `go version` of 1.26.3 is not the compiler identity of either oracle.
[File digests](evidence/decode-campaign-46acbcb-2026-11-12/digests.json)
bind all retained inputs to this report.

These counts concern decoding, signer extraction and wrapper parsing. They
do not establish mutation branch coverage, signed execution or delivery
saturation, consensus-operation volume, three clean nights, historical
replay, or a completed live observation window. The remaining F1 gates stay
open. Later source changes need their own candidate evidence. Both live
validators remained on candidate `46acbcb`; this work did not restart them.
