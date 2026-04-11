package dashboards

import (
	"fmt"

	cog "github.com/grafana/grafana-foundation-sdk/go/cog"
	"github.com/grafana/grafana-foundation-sdk/go/common"
	v2 "github.com/grafana/grafana-foundation-sdk/go/dashboardv2beta1"
	"github.com/grafana/grafana-foundation-sdk/go/stat"
	"github.com/grafana/grafana-foundation-sdk/go/timeseries"
)

// statLastNotNull returns a stat visualization showing the last non-null value.
func statLastNotNull() *stat.VisualizationBuilder {
	return stat.NewVisualizationBuilder().
		GraphMode(common.BigValueGraphModeArea).
		Orientation(common.VizOrientationAuto).
		ReduceOptions(common.NewReduceDataOptionsBuilder().Calcs([]string{"lastNotNull"}))
}

const performanceDefaultTitle = "Firewood: Performance"

// Performance generates the Firewood performance dashboard, replicating dashboard2.json.
// NOTE: This dashboard uses pre-overhaul metric names (pre-#1912). Do not update them.
type Performance struct{}

func NewPerformance() *Performance { return &Performance{} }

func (p *Performance) Name() string        { return "performance" }
func (p *Performance) Description() string { return "Firewood performance metrics dashboard" }

func (p *Performance) Build(opts BuildOptions) (v2.Dashboard, error) {
	b := v2.NewDashboardBuilder(resolveTitle(opts.TitleOverride, performanceDefaultTitle)).
		Editable(true).
		DatasourceVariable(v2.NewDatasourceVariableBuilder("datasource").
			Label("Data Source").
			PluginId("prometheus").
			AllowCustomValue(true).
			Hide(v2.VariableHideDontHide).
			Refresh(v2.VariableRefreshOnDashboardLoad),
		).
		AdhocVariable(v2.NewAdhocVariableBuilder("filter").
			Group("prometheus").
			Label("Filter").
			Datasource(v2.NewDashboardv2beta1AdhocVariableKindDatasourceBuilder().
				Name("${datasource}"),
			).
			Filters([]cog.Builder[v2.AdHocFilterWithLabels]{
				v2.NewAdHocFilterWithLabelsBuilder().Key("chain").Operator("=").Value("C").KeyLabel("chain").ValueLabels([]string{"C"}),
				v2.NewAdHocFilterWithLabelsBuilder().Key("namespace").Operator("=|").Value("avax-mainnet-api-firewood").KeyLabel("namespace").ValueLabels([]string{"avax-mainnet-api-firewood", "avax-mainnet-validator-firewood"}),
			}),
		)

	p.registerWorldStatsPanels(b)
	p.registerTopLevelLatencyPanels(b)
	p.registerReadPathPanels(b)
	p.registerWritePathPanels(b)
	p.registerLatencyPanels(b)
	p.registerAllocatorStoragePanels(b)
	p.registerWorkloadPanels(b)
	p.registerSystemHealthPanels(b)

	b.RowsLayout(v2.NewRowsBuilder().
		Row(p.buildWorldStatsRow()).
		Row(p.buildTopLevelLatencyRow()).
		Row(p.buildReadPathRow()).
		Row(p.buildWritePathRow()).
		Row(p.buildLatencyRow()).
		Row(p.buildAllocatorStorageRow()).
		Row(p.buildWorkloadRow()).
		Row(p.buildSystemHealthRow()),
	)

	return b.Build()
}

// ── World Stats ───────────────────────────────────────────────────────────────

func (p *Performance) registerWorldStatsPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-28", v2.NewPanelBuilder().Id(28).
		Title("Gas Processed").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_chain_block_gas_used_processed[5m]) / 1e6`, "__auto"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("si: mgas/s").
			Legend(legendOpts([]string{"min", "max", "mean", "sum", "stdDev", "variance"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-110", v2.NewPanelBuilder().Id(110).
		Title("Last Accepted Block").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A",
				"avalanche_snowman_last_accepted_height{\n  namespace=~\"avax-mainnet-(api|validator)-firewood\",\n  chain=\"C\"\n}",
				"__auto"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildWorldStatsRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("World Stats").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-28", 0, 0, 24, 10)).
			Item(gridItem("panel-110", 0, 10, 24, 10)))
}

// ── Top-Level Latency ─────────────────────────────────────────────────────────

func (p *Performance) registerTopLevelLatencyPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-14", v2.NewPanelBuilder().Id(14).
		Title("EVM vs Trie vs Write").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_chain_block_executions[5m])`, "EVM Execution")).
			Target(promQ("B", `rate(avalanche_evm_eth_chain_block_trie[5m])`, "Trie Ops")).
			Target(promQ("C", `rate(avalanche_evm_eth_chain_block_writes[5m])`, "Block Writes")).
			Target(promQ("D", `rate(avalanche_evm_eth_chain_block_executions[5m]) + rate(avalanche_evm_eth_chain_block_trie[5m]) + rate(avalanche_evm_eth_chain_block_writes[5m])`, "Sum(exec, trie, writes)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms").
			Legend(legendOpts([]string{"min", "max", "mean", "sum", "p50", "p95", "p99", "stdDev"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildTopLevelLatencyRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Top-Level Latency").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-14", 0, 0, 24, 10)))
}

// ── Read Path ─────────────────────────────────────────────────────────────────

func (p *Performance) registerReadPathPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-29", v2.NewPanelBuilder().Id(29).
		Title("State Reads: Go vs Firewood Disk").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_chain_account_reads[5m]) + rate(avalanche_evm_eth_chain_storage_reads[5m])`, "Go: State Reads (total)")).
			Target(promQ("B", `rate(avalanche_evm_firewood_io_read_ms[5m])`, "Firewood: Disk I/O"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms").
			Legend(legendOpts([]string{"min", "max", "mean", "p50", "p95", "p99", "stdDev"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-24", v2.NewPanelBuilder().Id(24).
		Title("Disk Read Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_io_read_ms[5m]) / rate(avalanche_evm_firewood_io_read[5m])`, "Avg Disk Read Latency (ms)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms").
			Legend(legendOpts([]string{"min", "max", "mean", "p50", "p95", "p99", "stdDev"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-32", v2.NewPanelBuilder().Id(32).
		Title("Node Cache Health").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A",
				"sum(rate(avalanche_evm_firewood_cache_node{type=\"hit\", mode=\"read\"}[5m])) \n/ \nsum(rate(avalanche_evm_firewood_cache_node{mode=\"read\"}[5m]))",
				"Read Cache Hit Rate")).
			Target(promQ("B",
				"sum(rate(avalanche_evm_firewood_cache_node{type=\"hit\", mode=\"write\"}[5m])) \n/ \nsum(rate(avalanche_evm_firewood_cache_node{mode=\"write\"}[5m]))",
				"Write Cache Hit Rate"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"min", "max", "mean", "sum", "stdDev", "variance"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-51", v2.NewPanelBuilder().Id(51).
		Title("Node Read Source (Disk vs In-Memory Proposals)").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_read_node{from="file"}[5m])`, "From disk (committed)")).
			Target(promQ("B", `rate(avalanche_evm_firewood_read_node{from="memory"}[5m])`, "From memory (proposals)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ops").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeMulti, common.SortOrderDescending))))

	b.Panel("panel-52", v2.NewPanelBuilder().Id(52).
		Title("Read Amplification (Physical I/O per Node Read)").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_io_read[5m]) / rate(avalanche_evm_firewood_read_node{from="file"}[5m])`, "Physical/Logical Read Ratio"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildReadPathRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Read Path").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-29", 0, 0, 12, 10)).
			Item(gridItem("panel-24", 12, 0, 12, 10)).
			Item(gridItem("panel-32", 0, 10, 12, 10)).
			Item(gridItem("panel-51", 12, 10, 12, 10)).
			Item(gridItem("panel-52", 0, 20, 12, 10)))
}

// ── Write Path ────────────────────────────────────────────────────────────────

func (p *Performance) registerWritePathPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-18", v2.NewPanelBuilder().Id(18).
		Title("Firewood Write Path: Propose vs Commit").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_ffi_commit_ms[5m])`, "Commit (disk persist)")).
			Target(promQ("B", `rate(avalanche_evm_firewood_ffi_propose_ms[5m])`, "Propose (trie update + hash)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms/s").
			Legend(legendOpts([]string{"min", "max", "mean", "sum", "stdDev", "variance"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-31", v2.NewPanelBuilder().Id(31).
		Title("Commit Stack").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_firewood_triedb_commit_time[5m])`, "Go: Commit")).
			Target(promQ("B", `rate(avalanche_evm_firewood_ffi_commit_ms[5m])`, "Rust: Commit (FFI)")).
			Target(promQ("C", `rate(avalanche_evm_firewood_flush_nodes[5m])`, "Disk I/O")).
			Target(promQ("D", `rate(avalanche_evm_firewood_flush_nodes[5m]) / rate(avalanche_evm_firewood_ffi_commit_ms[5m])`, "Disk I/O Efficiency"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms").
			Legend(legendOpts([]string{"min", "max", "mean", "p50", "p95", "p99", "stdDev"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-47", v2.NewPanelBuilder().Id(47).
		Title("Commit I/O Efficiency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_flush_nodes[5m]) / rate(avalanche_evm_firewood_ffi_commit_ms[5m])`, "Disk I/O fraction of commit"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("percentunit").
			Legend(legendOpts([]string{"mean", "min", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-108", v2.NewPanelBuilder().Id(108).
		Title("Commit Stall - Frequency and Duration").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `increase(avalanche_evm_firewood_persist_commit_blocked[5m])`, "Blocked Commits")).
			Target(promQ("B", `histogram_quantile(0.99, rate(avalanche_evm_firewood_ffi_commit_ms_bucket_bucket[5m]))`, "Commits p99"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"min", "max", "mean", "p95"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-107", v2.NewPanelBuilder().Id(107).
		Title("Permits Available").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_persist_permits_available`, "Permits Available - {{namespace}}")).
			Target(promQ("B", `avalanche_evm_firewood_persist_max_permits`, "Max Permits - {{namespace}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"min", "max", "mean", "p95", "lastNotNull"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-26", v2.NewPanelBuilder().Id(26).
		Title("Trie Mutations: Insert & Remove").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_insert[5m])`, "Insert — {{merkle}}")).
			Target(promQ("B", `rate(avalanche_evm_firewood_remove[5m])`, "Remove — prefix:{{prefix}} result:{{result}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ops").
			Legend(common.NewVizLegendOptionsBuilder().
				ShowLegend(true).
				DisplayMode(common.LegendDisplayModeTable).
				Placement(common.LegendPlacementBottom).
				Calcs([]string{"min", "max", "mean", "sum", "stdDev", "variance"}).
				SortBy("Total").
				SortDesc(true)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-57", v2.NewPanelBuilder().Id(57).
		Title("Proposal Lifecycle").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_proposals_created{base="db"}[5m])`, "Created from DB (committed base)")).
			Target(promQ("B", `rate(avalanche_evm_firewood_proposals_created{base="proposal"}[5m])`, "Created from Proposal (speculative)")).
			Target(promQ("C", `rate(avalanche_evm_firewood_proposals_reparented[5m])`, "Reparented (rebased after commit)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ops").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeMulti, common.SortOrderDescending))))

	b.Panel("panel-109", v2.NewPanelBuilder().Id(109).
		Title("Root Store: Insert").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A",
				"sum(rate(avalanche_evm_firewood_persist_root_store_ms{success=\"true\"}\n  [$__rate_interval]))\n    /\n    sum(rate(avalanche_evm_firewood_persist_root_store{success=\"true\"}[$__rate_interval]))",
				"Average Write Time (ms)"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildWritePathRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Write Path").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-18", 0, 0, 24, 11)).
			Item(gridItem("panel-31", 0, 11, 12, 10)).
			Item(gridItem("panel-47", 12, 11, 12, 10)).
			Item(gridItem("panel-108", 0, 21, 12, 10)).
			Item(gridItem("panel-107", 12, 21, 12, 10)).
			Item(gridItem("panel-26", 0, 31, 12, 10)).
			Item(gridItem("panel-57", 12, 31, 12, 10)).
			Item(gridItem("panel-109", 0, 41, 12, 8)))
}

// ── Latency Distributions ─────────────────────────────────────────────────────

func (p *Performance) registerLatencyPanels(b *v2.DashboardBuilder) {
	latencyDist := func(id int, op, metric string) {
		b.Panel(fmt.Sprintf("panel-%d", id), v2.NewPanelBuilder().Id(float64(id)).
			Title(op+" Latency Distribution (p50 / p95 / p99)").
			Data(v2.NewQueryGroupBuilder().
				Target(promQ("A", fmt.Sprintf(`histogram_quantile(0.50, sum by (le) (rate(%s[5m])))`, metric), op+" p50")).
				Target(promQ("B", fmt.Sprintf(`histogram_quantile(0.95, sum by (le) (rate(%s[5m])))`, metric), op+" p95")).
				Target(promQ("C", fmt.Sprintf(`histogram_quantile(0.99, sum by (le) (rate(%s[5m])))`, metric), op+" p99"))).
			Visualization(timeseries.NewVisualizationBuilder().Unit("ms").
				Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
				Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
	}
	latencyDist(41, "Commit", "avalanche_evm_firewood_ffi_commit_ms_bucket_bucket")
	latencyDist(42, "Propose", "avalanche_evm_firewood_ffi_propose_ms_bucket_bucket")
	latencyDist(54, "Batch", "avalanche_evm_firewood_ffi_batch_ms_bucket_bucket")

	b.Panel("panel-56", v2.NewPanelBuilder().Id(56).
		Title("Node Deletion by Area Size").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_delete_node[5m])`, "Delete area {{index}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ops").
			Legend(legendOpts([]string{"mean"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeMulti, common.SortOrderDescending))))
}

func (p *Performance) buildLatencyRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Latency Distributions").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-41", 0, 0, 12, 10)).
			Item(gridItem("panel-42", 12, 0, 12, 10)).
			Item(gridItem("panel-54", 0, 10, 12, 10)).
			Item(gridItem("panel-56", 12, 10, 12, 10)))
}

// ── Allocator & Storage ───────────────────────────────────────────────────────

func (p *Performance) registerAllocatorStoragePanels(b *v2.DashboardBuilder) {
	b.Panel("panel-27", v2.NewPanelBuilder().Id(27).
		Title("Allocator Reuse Efficiency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A",
				`sum by (index) (rate(avalanche_evm_firewood_space_reused[5m])) / (sum by (index) (rate(avalanche_evm_firewood_space_reused[5m])) + sum by (index) (rate(avalanche_evm_firewood_space_from_end[5m])))`,
				"Size class {{index}}"))).
		Visualization(statLastNotNull()))

	b.Panel("panel-33", v2.NewPanelBuilder().Id(33).
		Title("Freelist Cache Hit Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A",
				`sum(rate(avalanche_evm_firewood_cache_freelist{type="hit"}[5m])) / sum(rate(avalanche_evm_firewood_cache_freelist[5m]))`,
				"Freelist Cache Hit Rate"))).
		Visualization(statLastNotNull()))

	b.Panel("panel-58", v2.NewPanelBuilder().Id(58).
		Title("Allocator Space Flow (Reuse vs Growth)").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_space_reused[5m])`, "Reused (from free lists)")).
			Target(promQ("B", `rate(avalanche_evm_firewood_space_from_end[5m])`, "From End (file growth)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("decbytes").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeMulti, common.SortOrderDescending))))

	b.Panel("panel-111", v2.NewPanelBuilder().Id(111).
		Title("Jemalloc Memory Usage (gb) [Broken]").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_jemalloc_active_bytes / 1024 / 1024 / 1024`, "__auto"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildAllocatorStorageRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Allocator & Storage").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-27", 0, 0, 12, 8)).
			Item(gridItem("panel-33", 12, 0, 12, 8)).
			Item(gridItem("panel-58", 0, 8, 12, 10)).
			Item(gridItem("panel-111", 12, 8, 12, 10)))
}

// ── Workload Characterization ─────────────────────────────────────────────────

func (p *Performance) registerWorkloadPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-20", v2.NewPanelBuilder().Id(20).
		Title("Transaction Complexity").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_chain_block_gas_used_processed[5m]) / rate(avalanche_evm_eth_chain_txs_processed[5m])`, "Gas/Tx"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("gas/txs").
			Legend(legendOpts([]string{"min", "max", "mean", "sum", "stdDev", "variance"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-21", v2.NewPanelBuilder().Id(21).
		Title("State Access Pattern").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_eth_chain_storage_reads[5m]) / rate(avalanche_evm_eth_chain_account_reads[5m])`, "Storage:Account Ratio"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"mean", "stdDev", "variance"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildWorkloadRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("Workload Characterization").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-20", 0, 0, 12, 8)).
			Item(gridItem("panel-21", 12, 0, 12, 8)))
}

// ── System Health & Overhead ──────────────────────────────────────────────────

func (p *Performance) registerSystemHealthPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-44", v2.NewPanelBuilder().Id(44).
		Title("Go/CGO Overhead: Propose & Commit").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_triedb_hash_time[5m]) - rate(avalanche_evm_firewood_ffi_propose_ms[5m])`, "Propose: Go/CGO overhead")).
			Target(promQ("B", `rate(avalanche_evm_firewood_triedb_commit_time[5m]) - rate(avalanche_evm_firewood_ffi_commit_ms[5m])`, "Commit: Go/CGO overhead"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ms/s").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-45", v2.NewPanelBuilder().Id(45).
		Title("Proposal Queue & Revision Pressure").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_proposals_uncommitted`, "Uncommitted Proposals")).
			Target(promQ("B", `avalanche_evm_firewood_active_revisions`, "Active Revisions")).
			Target(promQ("C", `avalanche_evm_firewood_max_revisions`, "Max Revisions (config)"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"mean", "max", "last"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-46", v2.NewPanelBuilder().Id(46).
		Title("Error & Discard Rates").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_proposal_commit{success="false"}[5m])`, "Commit Failures")).
			Target(promQ("B", `rate(avalanche_evm_firewood_proposal_create{success="false"}[5m])`, "Propose Failures")).
			Target(promQ("C", `rate(avalanche_evm_firewood_proposals_discarded[5m])`, "Proposals Discarded"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ops").
			Legend(legendOpts([]string{"sum", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (p *Performance) buildSystemHealthRow() *v2.RowBuilder {
	return v2.NewRowBuilder().
		Title("System Health & Overhead").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-44", 0, 0, 12, 10)).
			Item(gridItem("panel-45", 12, 0, 12, 10)).
			Item(gridItem("panel-46", 0, 10, 12, 10)))
}
