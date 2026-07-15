package replay

import (
	"testing"

	"github.com/celestiaorg/celestia-app/v6/pkg/appconsts"
)

// TestNewCorpusAppBuildsAppV6WithBaseApp is the module smoke test: it proves the
// celestia-app v6 dependency graph (with the celestia cosmos-sdk fork replaces)
// compiles and that a full in-memory app, including its wired ante handler, can
// be constructed. Snapshot injection and ante execution are added on top of this
// in subsequent changes.
func TestNewCorpusAppBuildsAppV6WithBaseApp(t *testing.T) {
	application := NewCorpusApp()
	if application == nil {
		t.Fatal("NewCorpusApp returned nil")
	}
	if application.GetBaseApp() == nil {
		t.Fatal("celestia-app base app (which carries the ante handler) is nil")
	}
	if appconsts.Version != 6 {
		t.Fatalf("celestia-app appconsts.Version = %d, want app-v6", appconsts.Version)
	}
}
