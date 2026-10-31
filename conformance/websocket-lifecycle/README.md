# WebSocket lifecycle evidence

This recorder targets an isolated loopback Celestia node and writes one JSON
capture of the WebSocket lifecycle. It refuses non-loopback RPC and pprof
endpoints. The checked-in capture was made with:

- celestia-app `v10.5.0`, source commit
  `6013e758d5dceb75827ad191c8d739d7c43cfa04`
- celestia-core `v0.42.3` and Cosmos SDK `v0.52.12`
- the tagged and version-stamped oracle binary built by
  `conformance/devnet/build-app-oracle.sh`
- binary SHA-256
  `eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38`
- Go `1.26.9`

Immediately before collection, GitHub's latest release API and the remote tag
both resolved `v10.5.0` to the baseline commit. The command receipt is frozen
in `testdata/release.json`.

`testdata/manifest.json` SHA-256 binds both CORS captures, the raw mainnet
`status` and `abci_info` observations, release/build receipts, generated
oracle genesis and enabled/disabled configs, and every recorder/verifier
source. Run `python3 verify.py` to check the hashes, capture-time ordering,
mainnet chain/core/app/height agreement, quota invariants, and cleanup bounds.

The issue baseline recorded Go `1.26.6`. The release source and all relevant
module versions are identical; the builder patch version is retained in the
capture instead of being normalized away.

The evidence node used a three-client/five-subscription limit, 100-entry
subscription and socket queues, `experimental_close_on_slow_client = true`,
and CORS origin `https://allowed.example`. Reproduce both CORS modes, the
fresh release/mainnet receipts, generated configs, and the manifest with:

```sh
APPD=/path/to/the/exact/oracle ./record.sh
```

For an already-running equivalent node, the full recorder invocation is:

```sh
go run . \
  -rpc http://127.0.0.1:28657 \
  -pprof http://127.0.0.1:8060 \
  -clients 3 \
  -large-frame-bytes 16777216 \
  -appd /path/to/celestia-appd \
  -home /path/to/oracle-home \
  -chain-id ws190-105-1 \
  -node tcp://127.0.0.1:28657 \
  -out testdata/go-v10.5.0.json
```

The capture preserves full event payloads and their observed frame order.
Validation treats block heights and transaction hashes as values from this
run: reconnect behavior is checked relationally (the first resubscribed
height is after the disconnected interval). `Tx` and `NewBlock` are delivered
by independent subscription goroutines, so both must appear at the
transaction height while their relative frame order may interleave. The exact
observed order remains in the fixture. The Go
server's nominal 27-second ping and 30-second read deadline are accepted in
26–29 and 29–33 second windows. Goroutine cleanup is bounded relative to the
pre-test baseline, since the absolute runtime count depends on the node.

Successful upgrades deliberately omit `Access-Control-Allow-Origin` and
`Vary`, whether CORS is disabled or enabled and whether the Origin is allowed
or denied. The active-subscription probes show that Origin does not shorten
the connection lifetime. This follows the release transport's permissive
WebSocket `CheckOrigin`; HTTP preflight policy does not wrap successful
upgrades. The smaller `go-v10.5.0-cors-disabled.json` fixture was recorded
after restarting the same isolated node with `cors_allowed_origins = []` and
passing `-upgrades-only`.

The node's `max_body_bytes` was reduced to 1,000,000 for the large-frame
phase. The probe is a 16 MiB message, not a one-byte boundary probe. It is
rejected by closing the socket, without retaining the payload or returning a
JSON-RPC envelope.

The outbound-pressure phase sends large `genesis` responses without reading,
waits beyond the ten-second Go write deadline, checks that the connection is
closed, and then opens a healthy connection. The oversized-frame phase sends
a 16 MiB message against the 1,000,000-byte limit. Quota phases disconnect
clients and wait for cleanup before checking reclaimed capacity and pprof
goroutine bounds. The configured client quota is global and includes CometBFT's
internal event-bus clients, so the capture records the number of external
clients admitted instead of assuming all configured slots are externally
available.

This is a partial step for issue #190. It fixes and locally tests bounded
socket writes, idle reads, peer-task abort cleanup, subscription capacity
reclamation, and healthy reconnects. A longer multi-provider soak with
concurrent transaction bursts and OS-level memory/task telemetry remains for
the issue; the checked-in Go run bounds one lifecycle cycle and does not claim
that soak coverage.
