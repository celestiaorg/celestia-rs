package cmd

import (
	"reflect"
	"strings"
	"testing"

	"cosmossdk.io/store/mem"
)

func TestReplayStateMutationsDiffsSetUpdateAndDelete(t *testing.T) {
	before := mem.NewStore()
	before.Set([]byte("delete"), []byte("old"))
	before.Set([]byte("same"), []byte("value"))
	before.Set([]byte("update"), []byte("old"))

	after := mem.NewStore()
	after.Set([]byte("add"), []byte("new"))
	after.Set([]byte("same"), []byte("value"))
	after.Set([]byte("update"), []byte("new"))

	got, err := replayStateMutations("bank", before, after)
	if err != nil {
		t.Fatalf("replayStateMutations returned error: %v", err)
	}

	want := []multistoreMutation{
		{StoreName: "bank", Action: "set", KeyHex: "616464", ValueHex: "6e6577"},
		{StoreName: "bank", Action: "delete", KeyHex: "64656c657465"},
		{StoreName: "bank", Action: "set", KeyHex: "757064617465", ValueHex: "6e6577"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("mutations = %#v, want %#v", got, want)
	}
}

func TestReplayStateConfigValidation(t *testing.T) {
	_, err := parseReplayStateConfig([]string{"--start", "10", "--count", "2"})
	if err == nil || !strings.Contains(err.Error(), "--db-dir is required") {
		t.Fatalf("error = %v, want missing db-dir", err)
	}

	cfg, err := parseReplayStateConfig([]string{
		"--db-dir", t.TempDir(),
		"--start", "10",
		"--count", "2",
		"--mounts", "bank, auth,bank",
	})
	if err != nil {
		t.Fatalf("parseReplayStateConfig returned error: %v", err)
	}
	if want := []string{"bank", "auth"}; !reflect.DeepEqual(cfg.mounts, want) {
		t.Fatalf("mounts = %#v, want %#v", cfg.mounts, want)
	}
}

func TestReplayStateKeysAllowCelestiaStorePrefixes(t *testing.T) {
	keys := replayStateKeys([]string{"auth", "authz"})
	if keys["auth"].Name() != "auth" {
		t.Fatalf("auth key name = %q, want auth", keys["auth"].Name())
	}
	if keys["authz"].Name() != "authz" {
		t.Fatalf("authz key name = %q, want authz", keys["authz"].Name())
	}
}
