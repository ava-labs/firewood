package dashboards

import (
	cog "github.com/grafana/grafana-foundation-sdk/go/cog"
	"github.com/grafana/grafana-foundation-sdk/go/common"
	v2 "github.com/grafana/grafana-foundation-sdk/go/dashboardv2beta1"
)

// promQueryBuilder builds a PanelQueryKind for a Prometheus target.
type promQueryBuilder struct {
	refId  string
	expr   string
	legend string
	hidden bool
}

func (b promQueryBuilder) Build() (v2.PanelQueryKind, error) {
	ds := "${datasource}"
	return v2.PanelQueryKind{
		Kind: "PanelQuery",
		Spec: v2.PanelQuerySpec{
			RefId:  b.refId,
			Hidden: b.hidden,
			Query: v2.DataQueryKind{
				Kind:    "DataQuery",
				Group:   "prometheus",
				Version: "v0",
				Datasource: &v2.Dashboardv2beta1DataQueryKindDatasource{
					Name: &ds,
				},
				Spec: map[string]any{
					"expr":         b.expr,
					"legendFormat": b.legend,
				},
			},
		},
	}, nil
}

// promQ returns a builder for a visible Prometheus query.
func promQ(refId, expr, legend string) cog.Builder[v2.PanelQueryKind] {
	return promQueryBuilder{refId: refId, expr: expr, legend: legend}
}

// hiddenQ returns a builder for a hidden Prometheus query (used alongside math expressions).
func hiddenQ(refId, expr string) cog.Builder[v2.PanelQueryKind] {
	return promQueryBuilder{refId: refId, expr: expr, hidden: true}
}

// mathExprBuilder builds a PanelQueryKind for a Grafana math expression.
type mathExprBuilder struct {
	refId      string
	expression string
}

func (b mathExprBuilder) Build() (v2.PanelQueryKind, error) {
	return v2.PanelQueryKind{
		Kind: "PanelQuery",
		Spec: v2.PanelQuerySpec{
			RefId: b.refId,
			Query: v2.DataQueryKind{
				Kind:    "DataQuery",
				Group:   "__expr__",
				Version: "v0",
				Spec: map[string]any{
					"expression": b.expression,
					"type":       "math",
				},
			},
		},
	}, nil
}

// mathQ returns a builder for a math expression query.
func mathQ(refId, expression string) cog.Builder[v2.PanelQueryKind] {
	return mathExprBuilder{refId: refId, expression: expression}
}

// dataQueryBuilder builds a DataQueryKind for use in query variables.
type dataQueryBuilder struct {
	group string
	spec  any
}

func (b dataQueryBuilder) Build() (v2.DataQueryKind, error) {
	return v2.DataQueryKind{
		Kind:    "DataQuery",
		Group:   b.group,
		Version: "v0",
		Spec:    b.spec,
	}, nil
}

// legacyPromVar returns a DataQueryKind builder for a query variable using the legacy string format.
func legacyPromVar(query string) cog.Builder[v2.DataQueryKind] {
	return dataQueryBuilder{
		group: "prometheus",
		spec:  map[string]any{"__legacyStringValue": query},
	}
}

// gridItem returns a GridItemBuilder for the given element key and position.
func gridItem(name string, x, y, w, h int64) *v2.GridItemBuilder {
	return v2.NewGridItemBuilder().
		Name(name).
		X(x).Y(y).Width(w).Height(h)
}

// strOrMapStr wraps a plain string as a StringOrArrayOfString (the String variant).
func strOrMapStr(s string) v2.StringOrArrayOfString {
	return v2.StringOrArrayOfString{String: &s}
}

// resolveTitle returns override if non-empty, otherwise defaultTitle.
func resolveTitle(override, defaultTitle string) string {
	if override != "" {
		return override
	}
	return defaultTitle
}

// stackingPercent returns a StackingConfig builder set to percent mode.
func stackingPercent() cog.Builder[common.StackingConfig] {
	return common.NewStackingConfigBuilder().Mode(common.StackingModePercent)
}

// legendOpts returns a VizLegendOptions builder with the given calcs and display mode.
// Placement is always bottom; showLegend is always true.
func legendOpts(calcs []string, mode common.LegendDisplayMode) cog.Builder[common.VizLegendOptions] {
	if calcs == nil {
		calcs = []string{}
	}
	return common.NewVizLegendOptionsBuilder().
		ShowLegend(true).
		DisplayMode(mode).
		Placement(common.LegendPlacementBottom).
		Calcs(calcs)
}

// tooltipOpts returns a VizTooltipOptions builder with the given mode and sort order.
func tooltipOpts(mode common.TooltipDisplayMode, sort common.SortOrder) cog.Builder[common.VizTooltipOptions] {
	return common.NewVizTooltipOptionsBuilder().
		Mode(mode).
		Sort(sort)
}
