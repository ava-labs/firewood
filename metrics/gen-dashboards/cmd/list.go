package cmd

import (
	"fmt"
	"io"
	"text/tabwriter"

	"github.com/ava-labs/firewood/metrics/gen-dashboards/dashboards"
	"github.com/spf13/cobra"
)

func newListCmd(reg *dashboards.Registry) *cobra.Command {
	return &cobra.Command{
		Use:   "list",
		Short: "List all available dashboards",
		RunE: func(cmd *cobra.Command, args []string) error {
			return printDashboardList(reg, cmd.OutOrStdout())
		},
	}
}

func printDashboardList(reg *dashboards.Registry, out io.Writer) error {
	all := reg.All()
	if len(all) == 0 {
		fmt.Fprintln(out, "No dashboards registered.")
		return nil
	}
	w := tabwriter.NewWriter(out, 0, 0, 2, ' ', 0)
	fmt.Fprintln(w, "NAME\tDESCRIPTION")
	for _, d := range all {
		fmt.Fprintf(w, "%s\t%s\n", d.Name(), d.Description())
	}
	return w.Flush()
}
