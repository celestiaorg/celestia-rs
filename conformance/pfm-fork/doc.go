// Package pfmfork records what the Celestia fork of packet-forward-middleware
// (celestiaorg/ibc-apps#3) does with transfer memos, by running its real
// OnRecvPacket and OnTimeoutPacket with mocked keepers. See README.md.
package pfmfork

// ForkCommit is the fork commit that go.mod pins.
const ForkCommit = "106efffd6a4be3460e33ccf63b0e034b9be709f9"
