// gen-dashboards generates Grafana dashboard JSON for Firewood.
//
// Usage:
//
//	gen-dashboards                        # list all dashboards
//	gen-dashboards list                   # list all dashboards
//	gen-dashboards generate <name>        # generate JSON to stdout
//	gen-dashboards generate <name> -o f   # generate JSON to file f
//	gen-dashboards generate <name> --title "Custom Title"
//	gen-dashboards generate-all -d <dir>  # generate all dashboards into dir
//
// To regenerate and overwrite the checked-in JSON files from the repo root:
//
//	go run ./metrics/gen-dashboards generate cchain -o dashboard.json
//	go run ./metrics/gen-dashboards generate performance -o dashboard2.json
package main

import (
	"github.com/ava-labs/firewood/metrics/gen-dashboards/cmd"
	"github.com/ava-labs/firewood/metrics/gen-dashboards/dashboards"
)

func main() {
	reg := dashboards.NewRegistry()
	reg.Register(dashboards.NewCChain())
	reg.Register(dashboards.NewPerformance())
	cmd.Execute(reg)
}
