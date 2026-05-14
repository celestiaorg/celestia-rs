// fixturegen emits golden vectors used by celestia-rs conformance tests.
package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/rootulp/celestia-rs/conformance/fixturegen/cmd"
)

func main() {
	flag.Usage = func() {
		fmt.Fprintln(os.Stderr, "Usage: fixturegen <subcommand> [args...]")
		fmt.Fprintln(os.Stderr, "Subcommands:")
		for _, sc := range cmd.All {
			fmt.Fprintf(os.Stderr, "  %-22s %s\n", sc.Name, sc.Description)
		}
	}
	flag.Parse()

	args := flag.Args()
	if len(args) == 0 {
		flag.Usage()
		os.Exit(2)
	}
	name, rest := args[0], args[1:]

	for _, sc := range cmd.All {
		if sc.Name == name {
			if err := sc.Run(rest); err != nil {
				fmt.Fprintf(os.Stderr, "fixturegen %s: %v\n", name, err)
				os.Exit(1)
			}
			return
		}
	}
	fmt.Fprintf(os.Stderr, "unknown subcommand: %s\n", name)
	flag.Usage()
	os.Exit(2)
}
