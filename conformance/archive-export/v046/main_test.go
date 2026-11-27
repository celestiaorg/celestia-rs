package main

import "testing"

// The archive application.db has ~1.5M SST files. With pebble's defaults
// (1,000 open files, 8 MiB block cache) nearly every IAVL node read reopens
// a table and rereads its index and filter blocks.
func TestPebbleOptionsSizeTableAndBlockCaches(t *testing.T) {
	o := pebbleOptions(4096, 300000)
	if o.MaxOpenFiles != 300000 {
		t.Fatalf("MaxOpenFiles = %d, want 300000", o.MaxOpenFiles)
	}
	if o.Cache == nil || o.Cache.MaxSize() != 4096<<20 {
		t.Fatalf("block cache size = %v, want %d", o.Cache, 4096<<20)
	}
	if !o.DisableAutomaticCompactions {
		t.Fatal("automatic compactions must stay off: the export only reads")
	}
}
