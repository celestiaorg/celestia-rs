package main

// consensusOp: the consensus-layer ops run on the v10 oracle only (the
// node runs celestia-core v0.42.3's consensus at every app version).
func consensusOp(op string, _ []byte) out {
	return out{Err: "consensus op " + op + ": v10 oracle only"}
}
