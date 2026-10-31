# Comet HTTP boundary evidence (celestia-app v10.5.0)

This bundle records 43 bounded HTTP/1.1 cases from two isolated validators built from `github.com/celestiaorg/celestia-app/v10@v10.5.0`. The embedded Comet implementation is `github.com/celestiaorg/celestia-core@v0.42.3`, which reports protocol version `0.38.17`.

The release was selected only after a read-only `/status` and `/abci_info` observation of the active `celestia` network. The two responses agreed on height `15447466`, protocol app version `10`, and app release `10.5.0`. This establishes the release to test; it does not claim that the local configuration or responses are supplied by the public RPC provider.

## Reproduction inputs

The oracle build manifest binds the app module, source commit, Go 1.26.9 compiler, target environment, `ledger` tag, SDK/app linker stamps, and 184 MiB binary SHA-256. `oracle-build-info.txt.gz` preserves Go's complete module receipt. The binary is omitted from this repository because of its size; `provenance.json` records its path at capture and exact digest.

The isolated nodes were launched in separate terminals with:

```sh
./start-source.sh zero 6320 6321 0 4096 256
./start-source.sh custom 6330 6331 3 512 256
```

The six numeric arguments after the name are the RPC port, P2P port, maximum request batch size, maximum decoded body bytes, and maximum header bytes. The bundle freezes the resulting `config.toml`, common `app.toml`, public genesis, and a raw `/status` plus `/abci_info` identity for each node. Private validator keys, keyring material, and node databases are deliberately excluded.

After both nodes were healthy, the input corpus was executed once with:

```sh
python3 capture-source.py
python3 observe-local-source.py
```

`capture-source.py` is the complete request generator. Every request and response is also retained as raw base64 with its byte count and SHA-256, so the evidence does not depend on rerunning the oracle. The verifier decodes Content-Length and chunked requests, reparses every HTTP response, and checks the frozen 43-case name/category inventory.

Run the offline checks with:

```sh
python3 verify.py
python3 -m unittest -v test_verify.py
```

## Observed contract

- `max_request_batch_size = 3` accepts three elements and returns a JSON-RPC `-32600` response for four valid elements. Notification-only arrays count toward the limit. A four-element array containing a scalar does not pass the typed batch precheck and later becomes a handler `-32700` parse error.
- `max_request_batch_size = 0` disables the batch-count precheck. Four and fifty request arrays were accepted.
- A lone notification, notification-only batch, and empty batch return HTTP 200 with an empty body. A mixed notification batch returns only the response-bearing item.
- A decoded body limit of 512 accepts 511 and 512 bytes and rejects 513 bytes with HTTP 400 for both Content-Length and chunked transfer coding.
- With a nonzero batch limit, body and batch prechecks are outside the CORS wrapper. Their errors therefore precede preflight, route selection, and CORS headers. With batch limit zero, an OPTIONS handler can return 204 without reading an over-limit body, so the MaxBytesReader limit is never triggered.
- The configured `max_header_bytes = 256` accepted a 4,352-byte raw request and rejected the next 4,353-byte request with HTTP 431. This reflects Go `net/http`'s 4 KiB read-buffer allowance; all thirteen binary-search probes are retained.
- A successful request and a batch-limit error both permit another request on the same HTTP/1.1 connection. A decoded-body overflow returns `Connection: close`, and the second request receives EOF.

Stable header projections exclude exactly `Connection`, `Content-Length`, `Date`, `Transfer-Encoding`, and `X-Server-Time`. The raw exchanges still retain these fields. Header absence is part of the precedence assertions, and connection closure is checked separately.

## Deliberate limits

This is partial evidence for issue #191. It does not cover reverse proxies, forwarded headers, TLS termination, malformed HTTP framing or conflicting length fields, HTTP/2 header-list behavior, slow-header/body timeouts, socket-level resource growth, or every possible header shape. It also does not establish public-provider parity for the isolated CORS and byte-limit configuration. Those areas remain open unless separate evidence covers them.

The paired Rust implementation uses Hyper's native HTTP/1 header-size enforcement at the observed 4,352-byte cutoff. Hyper returns the same 431 status and closes the connection, but its automatic status-only response has an empty body and omits Go's `Content-Type: text/plain; charset=utf-8` header instead of returning Go's `431 Request Header Fields Too Large` diagnostic. Reimplementing HTTP framing solely to reproduce that text response would add a second parser and change behavior in the malformed and slow-framing areas that this campaign did not test. The empty 431 body and absent diagnostic content type are therefore explicit Rust deviations; status, threshold, and connection closure remain the compatibility contract.
