# Native DA WebSocket transport

This is the MIT-licensed source of `libp2p-websocket` 0.46.0, from rust-libp2p commit `7171dce2f90c05ba7892d4ba926abb1881db27c7`. The source files contain the upstream license notices. The local change replaces the compiled Mozilla root certificate dataset with the platform trust store in `src/tls.rs`, matching the Go DA host and respecting operator CA overrides. Recheck that change when updating the transport.

[Upstream source](https://github.com/libp2p/rust-libp2p/tree/7171dce2f90c05ba7892d4ba926abb1881db27c7/transports/websocket)
