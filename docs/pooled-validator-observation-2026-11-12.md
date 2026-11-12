# Parallel pooled-reference validator observation — 2026-11-12

A new read-only observation window began on both local validators at
**2026-11-12 05:23:57 UTC**, using the stronger result-commitment collector
and pooled HTTPS reference transport. The installed validator candidate
remains `46acbcb516c3dab26cb5706c908dfe0207f27349`; Mocha PID 26124 and
mainnet PID 50523 continued running. The original collectors also continue
writing their original files. Their retained failures remain unknown periods
under the existing audit rules.

The original collectors' 05:18 UTC inspection found six unavailable Mocha
status samples and nine unavailable height attempts, plus ten unavailable
mainnet status samples and seven unavailable height attempts. TLS handshake
timeouts predominated. Reusing reference HTTPS connections targets that
transport problem; it does not reinterpret previous failures as successes.

Before durable installation, separate 120-second pooled probes completed:

| Network | Inclusive executed heights | Matching heights | Transactions | Observed samples |
|---|---|---:|---:|---:|
| Mocha `mocha-5` | 2,542,969–2,543,008 | 40 | 41 | 24 |
| Mainnet `celestia` | 15,815,522–15,815,559 | 38 | 314 | 24 |

Neither probe retained an unavailable attempt. Every recorded transaction
result commitment was recomputed from the archived normalized lists and
matched H+1's reference commitment. Both controllers finished with unchanged
input hashes. This is a bounded transport/collector check, rather than evidence
that the public providers will remain available for three days.

[Retained evidence](evidence/pooled-validator-observation-2026-11-12/canary-summary.json)
contains compressed probe events, samples and heights, controller outcomes,
public configurations, durable baselines and LaunchAgent files.
[Digests](evidence/pooled-validator-observation-2026-11-12/digests.json) identify
all retained files. The [installation manifest](evidence/pooled-validator-observation-2026-11-12/installation.json)
records fixed tool hashes and the exact deployment-hold transition.

The [observer fix](https://github.com/celestiaorg/celestia-rs/pull/367) is
revision `a2fa70b3755f0f6b184facb81c15851caeadc24b`. The
[read-only relay](https://github.com/celestiaorg/celestia-rs/pull/365) is built
from `baf29ffab66910fe1a4f53a9b945f3bfd7f58e35` using Go 1.26.10, with
executable SHA-256
`29a9cbe9ef6060235c2d2e3de29a0e064d0516913129365d105fe3e1577cd85d`.
Collector tool source is independent of the validator candidate identity.

The fixed installation is
`~/.celestia-rs/auto-update/qualification-v3-pooled-2026-11-12/`.
Each Background/Nice 10 LaunchAgent owns one controller, one loopback relay
and one observer. The relay permits read routes only, keeps certificate
verification enabled, and caps each provider at four request starts per
second and four connections. Recorded ready metadata binds each loopback
reference URL to its public upstream: `https://rpc.celestia-mocha.com` for
Mocha and `https://rpc.celestia.pops.one` for mainnet. The collector checks
that reference's chain identity. This transport does not independently
verify reference commit signatures.

The [deployed controller](evidence/pooled-validator-observation-2026-11-12/run.py)
stops its own observer if its relay exits, bounds the collector lifetime,
and terminates only its own child processes on exit. `KeepAlive` is false;
a failed run cannot silently restart into its existing output directory.
Fixed source copies sit outside development checkouts. The standard-library
plist installer used system Python; runtime collection uses Python 3.14.

The separately required first executed heights are **2,543,053** on Mocha
and **15,815,604** on mainnet. Seventy-two hours cannot elapse before
**November 15 at 05:23:57 UTC**. Collection is scheduled for 259,500 seconds,
ending around 05:28:57 UTC. The existing deployment hold was extended from
November 15 at 01:29:36 UTC to **06:28:57 UTC**, leaving an hour after the
new collector deadline. The watchdog/health services remain active.

Final review must retain all attempts, verify interval edges and every-height
coverage, process/binary continuity, transaction commitments, participation
and independent monitoring. Any unavailable records remain explicit gaps.
No three-day, historical replay or production qualification is established
by this installation, and #299/#302 remain open.
