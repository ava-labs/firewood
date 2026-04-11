package cmd

import (
	"github.com/ava-labs/firewood/metrics/gen-dashboards/dashboards"
	"github.com/spf13/cobra"
)

// Execute builds the cobra command tree and runs it.
func Execute(reg *dashboards.Registry) {
	root := &cobra.Command{
		Use:   "gen-dashboards",
		Short: "Generate Grafana dashboard JSON for Firewood",
		// Default to list behaviour when no subcommand is given.
		RunE: func(cmd *cobra.Command, args []string) error {
			return printDashboardList(reg, cmd.OutOrStdout())
		},
	}

	root.AddCommand(newListCmd(reg))
	root.AddCommand(newGenerateCmd(reg))
	root.AddCommand(newGenerateAllCmd(reg))

	if err := root.Execute(); err != nil {
		// cobra already prints the error; non-zero exit is handled by os.Exit.
	}
}
