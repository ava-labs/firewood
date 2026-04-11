package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/ava-labs/firewood/metrics/gen-dashboards/dashboards"
	"github.com/spf13/cobra"
)

func newGenerateAllCmd(reg *dashboards.Registry) *cobra.Command {
	var outputDir string

	cmd := &cobra.Command{
		Use:   "generate-all",
		Short: "Generate all dashboards, one JSON file per dashboard",
		RunE: func(cmd *cobra.Command, args []string) error {
			if outputDir == "" {
				return fmt.Errorf("--dir is required")
			}
			if err := os.MkdirAll(outputDir, 0o755); err != nil {
				return fmt.Errorf("creating output directory %s: %w", outputDir, err)
			}

			for _, d := range reg.All() {
				built, err := d.Build(dashboards.BuildOptions{})
				if err != nil {
					return fmt.Errorf("building dashboard %q: %w", d.Name(), err)
				}
				data, err := json.MarshalIndent(built, "", "  ")
				if err != nil {
					return fmt.Errorf("serializing dashboard %q: %w", d.Name(), err)
				}
				path := filepath.Join(outputDir, d.Name()+".json")
				if err := writeDashboardJSON(path, data); err != nil {
					return err
				}
				fmt.Fprintf(cmd.OutOrStderr(), "wrote %s\n", path)
			}
			return nil
		},
	}

	cmd.Flags().StringVarP(&outputDir, "dir", "d", "", "directory to write dashboard JSON files into (required)")

	return cmd
}
