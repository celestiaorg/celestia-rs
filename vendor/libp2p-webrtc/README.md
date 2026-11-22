This directory contains the MIT-licensed native WebRTC transport from rust-libp2p 0.10.0-alpha. The source files retain the upstream license notices. The local change enables a loopback ICE candidate for loopback peers and restricts the single-candidate workaround to the matching address class. This supports private and same-host DA peers without gathering a loopback candidate for a remote peer.

[Upstream source](https://github.com/libp2p/rust-libp2p/tree/7171dce2f90c05ba7892d4ba926abb1881db27c7/transports/webrtc)
