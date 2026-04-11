package cmd

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/ava-labs/firewood/metrics/gen-dashboards/dashboards"
	"github.com/spf13/cobra"
)

func newGenerateCmd(reg *dashboards.Registry) *cobra.Command {
	var (
		outputPath    string
		titleOverride string
	)

	cmd := &cobra.Command{
		Use:   "generate <name>",
		Short: "Generate a single dashboard as JSON",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			name := args[0]
			d, ok := reg.Get(name)
			if !ok {
				return fmt.Errorf("unknown dashboard %q (run 'list' to see available dashboards)", name)
			}

			opts := dashboards.BuildOptions{TitleOverride: titleOverride}
			built, err := d.Build(opts)
			if err != nil {
				return fmt.Errorf("building dashboard %q: %w", name, err)
			}

			data, err := json.MarshalIndent(built, "", "  ")
			if err != nil {
				return fmt.Errorf("serializing dashboard %q: %w", name, err)
			}

			if outputPath == "" {
				w := cmd.OutOrStdout()
				if _, err = w.Write(data); err != nil {
					return err
				}
				_, err = fmt.Fprintln(w)
				return err
			}

			if err := writeDashboardJSON(outputPath, data); err != nil {
				return err
			}
			fmt.Fprintf(cmd.OutOrStderr(), "wrote %s\n", outputPath)
			return nil
		},
	}

	cmd.Flags().StringVarP(&outputPath, "output", "o", "", "write JSON to this file instead of stdout")
	cmd.Flags().StringVar(&titleOverride, "title", "", "override the dashboard title")

	return cmd
}

func writeDashboardJSON(path string, data []byte) error {
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		return fmt.Errorf("writing %s: %w", path, err)
	}
	return nil
}
