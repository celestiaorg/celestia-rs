// Package pfmfork records what the Celestia fork of packet-forward-middleware
// (celestiaorg/ibc-apps#3) does with transfer memos, by running its real
// OnRecvPacket and OnTimeoutPacket with mocked keepers. See README.md.
package pfmfork

// ForkCommit is the fork commit that go.mod pins.
const ForkCommit = "34b056cdd1ea870aa95d8c52ebc9c8032b849afd"
