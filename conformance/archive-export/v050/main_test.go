package main

import "testing"

// The archive application.db has ~1.5M SST files; pebble's default table
// cache (1,000 open files) makes almost every IAVL node read reopen an SST.
func TestDBOptionsRaiseMaxOpenFiles(t *testing.T) {
	if got := dbOptions(300000).Get("maxopenfiles"); got != 300000 {
		t.Fatalf("maxopenfiles = %v, want 300000", got)
	}
}
