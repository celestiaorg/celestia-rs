// Package cmd hosts the fixturegen subcommands.
//
// celestia-app source version pinned for fixture generation: cosmos-sdk v0.50.10.
// Update this comment when upgrading.
package cmd

import (
	"flag"
	"fmt"
)

// Subcommand is the contract every fixturegen subcommand satisfies.
type Subcommand struct {
	Name        string
	Description string
	Run         func(args []string) error
}

// All registered subcommands, in display order.
var All = []Subcommand{
	{
		Name:        "bech32-addresses",
		Description: "Emit canonical celestia* bech32 addresses for hardcoded test pubkeys.",
		Run:         runBech32,
	},
}

func runBech32(args []string) error {
	fs := flag.NewFlagSet("bech32-addresses", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/_bootstrap/bech32-addresses.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if err := EmitBech32(*out); err != nil {
		return fmt.Errorf("emit bech32: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}
