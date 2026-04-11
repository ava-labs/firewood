package dashboards

import (
	"fmt"

	cog "github.com/grafana/grafana-foundation-sdk/go/cog"
	"github.com/grafana/grafana-foundation-sdk/go/common"
	v2 "github.com/grafana/grafana-foundation-sdk/go/dashboardv2beta1"
	"github.com/grafana/grafana-foundation-sdk/go/timeseries"
)

const cchainDefaultTitle = "Firewood: C-Chain Metrics"

// CChain generates the C-Chain metrics dashboard, replicating dashboard.json.
type CChain struct{}

func NewCChain() *CChain { return &CChain{} }

func (c *CChain) Name() string        { return "cchain" }
func (c *CChain) Description() string { return "Firewood C-Chain metrics (Avalanche EVM + Firewood)" }

func (c *CChain) Build(opts BuildOptions) (v2.Dashboard, error) {
	b := v2.NewDashboardBuilder(resolveTitle(opts.TitleOverride, cchainDefaultTitle)).
		Editable(true).
		TimeSettings(v2.NewTimeSettingsBuilder().
			From("now-24h").To("now").
			Timezone("browser").
			AutoRefresh("10s").
			AutoRefreshIntervals([]string{"5s", "10s", "30s", "1m", "5m", "15m", "30m", "1h", "2h", "1d"}),
		).
		// Variables
		DatasourceVariable(v2.NewDatasourceVariableBuilder("datasource").
			Label("Data Source").
			PluginId("prometheus").
			Refresh(v2.VariableRefreshOnDashboardLoad).
			AllowCustomValue(true),
		).
		AdhocVariable(v2.NewAdhocVariableBuilder("filter").
			Group("prometheus").
			Label("Filter").
			Datasource(v2.NewDashboardv2beta1AdhocVariableKindDatasourceBuilder().
				Name("${datasource}"),
			).
			Filters([]cog.Builder[v2.AdHocFilterWithLabels]{
				v2.NewAdHocFilterWithLabelsBuilder().Key("is_ephemeral_node").Operator("=").Value("false").KeyLabel("is_ephemeral_node").ValueLabels([]string{"false"}),
				v2.NewAdHocFilterWithLabelsBuilder().Key("gh_repo").Operator("=").Value("ava-labs/avalanchego").KeyLabel("gh_repo").ValueLabels([]string{"ava-labs/avalanchego"}),
				v2.NewAdHocFilterWithLabelsBuilder().Key("gh_job_id").Operator("=").Value("c-chain-reexecution").KeyLabel("gh_job_id").ValueLabels([]string{"c-chain-reexecution"}),
				v2.NewAdHocFilterWithLabelsBuilder().Key("config").Operator("=").Value("firewood").KeyLabel("config").ValueLabels([]string{"firewood"}),
			}),
		).
		QueryVariable(v2.NewQueryVariableBuilder("chain").
			Query(legacyPromVar("label_values(avalanche_evm_eth_chain_account_commits, chain)")).
			Refresh(v2.VariableRefreshOnDashboardLoad).
			AllowCustomValue(true).
			Current(v2.VariableOption{Text: strOrMapStr("C"), Value: strOrMapStr("C")}),
		)

	// Register all panel elements.
	c.registerAvalancheEVMPanels(b)
	c.registerWritePathPanels(b)
	c.registerReadPathPanels(b)
	c.registerLatencyPanels(b)
	c.registerStorageIOPanels(b)
	c.registerFFIProofPanels(b)
	c.registerMemoryPanels(b)
	c.registerSpaceAllocationPanels(b)

	// Build the rows layout.
	b.RowsLayout(v2.NewRowsBuilder().
		Row(c.buildAvalancheEVMRow()).
		Row(c.buildWritePathRow()).
		Row(c.buildReadPathRow()).
		Row(c.buildLatencyRow()).
		Row(c.buildStorageIORow()).
		Row(c.buildFFIProofRow()).
		Row(c.buildMemoryRow()).
		Row(c.buildSpaceAllocationRow()),
	)

	return b.Build()
}

// ── Avalanche EVM row ────────────────────────────────────────────────────────

func (c *CChain) registerAvalancheEVMPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-7", v2.NewPanelBuilder().Id(7).
		Title("Last Accepted Height").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_snowman_last_accepted_height{chain="$chain"}`, "__auto"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-3", v2.NewPanelBuilder().Id(3).
		Title("Verified MGas/s").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (chain) (rate(avalanche_evm_eth_chain_block_gas_used_processed{chain="${chain}"}[1m])) / 1000000`, "MGas / s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	const pctDenom = ` + rate(avalanche_evm_eth_chain_block_inits_state{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_block_executions{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_block_validations_state{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_block_trie{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_block_writes{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_account_commits{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_storage_commits{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_snapshot_commits{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_triedb_commits{chain="${chain}"}[1m]) + rate(avalanche_evm_eth_chain_block_signature_recovery{chain="${chain}"}[1m])) + 0.000001)`
	pctOf := func(num string) string {
		return fmt.Sprintf(`sum by (chain) (rate(%s{chain="${chain}"}[1m])) / (sum by (chain) (rate(avalanche_evm_eth_chain_block_validations_content{chain="${chain}"}[1m])%s`, num, pctDenom)
	}
	b.Panel("panel-2", v2.NewPanelBuilder().Id(2).
		Title("Block Processing %").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", pctOf("avalanche_evm_eth_chain_block_validations_content"), "Content Validation %")).
			Target(promQ("B", pctOf("avalanche_evm_eth_chain_block_inits_state"), "State Init %")).
			Target(promQ("C", pctOf("avalanche_evm_eth_chain_block_executions"), "Execution %")).
			Target(promQ("D", pctOf("avalanche_evm_eth_chain_block_validations_state"), "State Validation %")).
			Target(promQ("E", pctOf("avalanche_evm_eth_chain_block_trie"), "Trie Ops %")).
			Target(promQ("F", pctOf("avalanche_evm_eth_chain_block_writes"), "Write %")).
			Target(promQ("G", `(sum by (chain) (rate(avalanche_evm_eth_chain_account_commits{chain="${chain}"}[1m])) + sum by (chain) (rate(avalanche_evm_eth_chain_storage_commits{chain="${chain}"}[1m])) + sum by (chain) (rate(avalanche_evm_eth_chain_snapshot_commits{chain="${chain}"}[1m])) + sum by (chain) (rate(avalanche_evm_eth_chain_triedb_commits{chain="${chain}"}[1m]))) / (sum by (chain) (rate(avalanche_evm_eth_chain_block_validations_content{chain="${chain}"}[1m])`+pctDenom, "Commit Total %")).
			Target(promQ("H", pctOf("avalanche_evm_eth_chain_block_signature_recovery"), "Signature Recovery %"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("percentunit").
			FillOpacity(50).
			Stacking(stackingPercent()).
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	const commitDenom = ` + sum by (chain) (rate(avalanche_evm_eth_chain_storage_commits{chain="${chain}"}[1m])) + sum by (chain) (rate(avalanche_evm_eth_chain_snapshot_commits{chain="${chain}"}[1m])) + sum by (chain) (rate(avalanche_evm_eth_chain_triedb_commits{chain="${chain}"}[1m])) + 0.000001)`
	commitPct := func(metric string) string {
		return fmt.Sprintf(`sum by (chain) (rate(%s{chain="${chain}"}[1m])) / (sum by (chain) (rate(avalanche_evm_eth_chain_account_commits{chain="${chain}"}[1m]))%s`, metric, commitDenom)
	}
	b.Panel("panel-1", v2.NewPanelBuilder().Id(1).
		Title("Commit Time").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", commitPct("avalanche_evm_eth_chain_account_commits"), "Account Commit %")).
			Target(promQ("B", commitPct("avalanche_evm_eth_chain_storage_commits"), "Storage Commit %")).
			Target(promQ("C", commitPct("avalanche_evm_eth_chain_snapshot_commits"), "Snapshot Commit %")).
			Target(promQ("D", commitPct("avalanche_evm_eth_chain_triedb_commits"), "TrieDB Commit %"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("percentunit").
			FillOpacity(50).
			Stacking(stackingPercent()).
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-6", v2.NewPanelBuilder().Id(6).
		Title("EVM Accepted TPS").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (chain) (rate(avalanche_evm_eth_chain_txs_accepted{chain="${chain}"}[1m]))`, "EVM Accepted TPS"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-5", v2.NewPanelBuilder().Id(5).
		Title("Blocks Verified / S").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (chain) (rate(avalanche_evm_eth_chain_block_inserts_count{chain="${chain}"}[1m]))`, "Blocks Verified / S"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-4", v2.NewPanelBuilder().Id(4).
		Title("Txs / Block").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (chain) (rate(avalanche_evm_eth_chain_txs_processed{chain="${chain}"}[1m])) / (sum by (chain) (rate(avalanche_evm_eth_chain_block_inserts_count{chain="${chain}"}[1m])) + 0.000001)`, "Txs / Block"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-20", v2.NewPanelBuilder().Id(20).
		Title("Rejected Blocks in Last Minute").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `round(increase(avalanche_snowman_blks_rejected_count{chain="$chain", job="avalanchego"}[1m]))>0`, "Blocks {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("short").
			FillOpacity(100).
			Legend(legendOpts([]string{"min", "mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-15", v2.NewPanelBuilder().Id(15).
		Title("Accepted Blocks in Last Minute").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `round(increase(avalanche_snowman_blks_accepted_count{chain="$chain", job="avalanchego"}[1m]))>0`, "Blocks {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("short").
			FillOpacity(100).
			Legend(legendOpts([]string{"min", "mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-23", v2.NewPanelBuilder().Id(23).
		Title("Average Block Rejection Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_snowman_blks_rejected_sum{chain="$chain", job="avalanchego"}[5m]) / rate(avalanche_snowman_blks_rejected_count{chain="$chain", job="avalanchego"}[5m])`, "Avg Rejection Latency {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ns").
			Legend(legendOpts([]string{"min", "mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-22", v2.NewPanelBuilder().Id(22).
		Title("Average Block Acceptance Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_snowman_blks_accepted_sum{chain="$chain", job="avalanchego"}[5m]) / rate(avalanche_snowman_blks_accepted_count{chain="$chain", job="avalanchego"}[5m])`, "Avg Acceptance Latency {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ns").
			Legend(legendOpts([]string{"min", "mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-39", v2.NewPanelBuilder().Id(39).
		Title("Incomplete Polls").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_snowman_polls{chain="$chain"} > 0`, "Polls {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("short").FillOpacity(100).
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-9", v2.NewPanelBuilder().Id(9).
		Title("Processing Blocks").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_snowman_blks_processing{chain="$chain", job="avalanchego"}>0`, "Blocks {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Unit("short").FillOpacity(100).
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-11", v2.NewPanelBuilder().Id(11).
		Title("Percentage of Successful Queries").
		Data(v2.NewQueryGroupBuilder().
			Target(hiddenQ("A", `max(increase(avalanche_handler_messages{chain="$chain", op="chits", job="avalanchego"}[5m]))`)).
			Target(hiddenQ("B", `max(increase(avalanche_handler_messages{chain="$chain", op="query_failed", job="avalanchego"}[5m]))`)).
			Target(mathQ("% Successful", "($A + 1) / ($A + $B + 1)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("percentunit").
			Legend(legendOpts([]string{"min", "mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-40", v2.NewPanelBuilder().Id(40).
		Title("Message Handling Time (Total)").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_handler_message_handling_time{chain="$chain", job="avalanchego"}[5m])`, "{{ op }} {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ns").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-41", v2.NewPanelBuilder().Id(41).
		Title("Message Handling Time (per Message)").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_handler_message_handling_time{chain="$chain", job="avalanchego"}[5m])/rate(avalanche_handler_messages{chain="$chain", job="avalanchego"}[5m])`, "{{ op }} {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("ns").
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-19", v2.NewPanelBuilder().Id(19).
		Title("Unprocessed Incoming Messages").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_handler_sync_unprocessed_msgs_count{chain="$chain"}`, "{{ op }} {{node_id}}")).
			Target(promQ("B", `avalanche_handler_async_unprocessed_msgs_count{chain="$chain"}`, "{{ op }}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-29", v2.NewPanelBuilder().Id(29).
		Title("Incoming Messages Expired in Last Minute").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `increase(avalanche_handler_expired{chain="$chain", job="avalanchego"}[1m]) or 0 * up`, "{{ op }} {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-42", v2.NewPanelBuilder().Id(42).
		Title("Messages Received per Second").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_handler_messages{chain="$chain", job="avalanchego"}[5m])`, "{{ op }} {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeTable)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-25", v2.NewPanelBuilder().Id(25).
		Title("AVAX Benched").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avg_over_time(avalanche_benchlist_benched_weight{chain="$chain", job="avalanchego"}[15m]) / 10^9`, "AVAX Benched {{node_id}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts([]string{"mean", "max"}, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	// Cache hit rate panels use math expressions.
	cacheHitRate := func(key, hitExpr, totalExpr string) *v2.PanelBuilder {
		return v2.NewPanelBuilder().
			Title(key).
			Data(v2.NewQueryGroupBuilder().
				Target(hiddenQ("A", hitExpr)).
				Target(hiddenQ("B", totalExpr)).
				Target(mathQ("Hit Rate", "$A/$B"))).
			Visualization(timeseries.NewVisualizationBuilder().Unit("percentunit").
				Legend(legendOpts(nil, common.LegendDisplayModeList)).
				Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone)))
	}

	b.Panel("panel-36", cacheHitRate("Decided Cache Hit Rate",
		`sum(increase(avalanche_evm_chain_state_decided_cache_get_count{chain="$chain", result="hit"}[5m]) or 0 * up)`,
		`sum(increase(avalanche_evm_chain_state_decided_cache_get_count{chain="$chain"}[5m]) or 0 * up)`).Id(36))

	b.Panel("panel-35", cacheHitRate("Block ID Cache Hit Rate",
		`sum(increase(avalanche_evm_chain_state_bytes_to_id_cache_get_count{chain="$chain", result="hit"}[5m]) or 0 * up)`,
		`sum(increase(avalanche_evm_chain_state_bytes_to_id_cache_get_count{chain="$chain"}[5m]) or 0 * up)`).Id(35))

	b.Panel("panel-38", cacheHitRate("Unverified Block Cache Hit Rate",
		`sum(increase(avalanche_evm_chain_state_unverified_cache_get_count{chain="$chain", result="hit"}[5m]) or 0 * up)`,
		`sum(increase(avalanche_evm_chain_state_unverified_cache_get_count{chain="$chain"}[5m]) or 0 * up)`).Id(38))

	b.Panel("panel-37", cacheHitRate("Missing Block Cache Hit Rate",
		`sum(increase(avalanche_evm_chain_state_missing_cache_get_count{chain="$chain", result="hit"}[5m]) or 0 * up)`,
		`sum(increase(avalanche_evm_chain_state_missing_cache_get_count{chain="$chain"}[5m]) or 0 * up)`).Id(37))
}

func (c *CChain) buildAvalancheEVMRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Avalanche EVM").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-7", 0, 0, 12, 8)).
			Item(gridItem("panel-3", 12, 0, 12, 9)).
			Item(gridItem("panel-2", 0, 8, 12, 9)).
			Item(gridItem("panel-1", 12, 9, 12, 9)).
			Item(gridItem("panel-6", 0, 17, 12, 9)).
			Item(gridItem("panel-5", 12, 18, 12, 9)).
			Item(gridItem("panel-4", 0, 26, 12, 9)).
			Item(gridItem("panel-20", 12, 27, 12, 8)).
			Item(gridItem("panel-15", 0, 35, 12, 8)).
			Item(gridItem("panel-23", 12, 35, 12, 8)).
			Item(gridItem("panel-22", 0, 43, 12, 8)).
			Item(gridItem("panel-39", 12, 43, 12, 8)).
			Item(gridItem("panel-9", 0, 51, 12, 8)).
			Item(gridItem("panel-11", 12, 51, 12, 8)).
			Item(gridItem("panel-40", 0, 59, 12, 9)).
			Item(gridItem("panel-41", 12, 59, 12, 9)).
			Item(gridItem("panel-19", 0, 68, 6, 8)).
			Item(gridItem("panel-29", 6, 68, 6, 8)).
			Item(gridItem("panel-42", 12, 68, 12, 9)).
			Item(gridItem("panel-25", 0, 76, 12, 9)).
			Item(gridItem("panel-36", 12, 77, 12, 8)).
			Item(gridItem("panel-35", 0, 85, 12, 8)).
			Item(gridItem("panel-38", 12, 85, 12, 8)).
			Item(gridItem("panel-37", 0, 93, 12, 8)))
}

// ── Write Path row ───────────────────────────────────────────────────────────

func (c *CChain) registerWritePathPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-44", v2.NewPanelBuilder().Id(44).
		Title("Firewood: Proposal Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (base) (rate(avalanche_evm_firewood_firewood_proposals_total[$__rate_interval]))`, "Proposed ({{base}})")).
			Target(promQ("B", `sum by (success) (rate(avalanche_evm_firewood_firewood_proposal_commits_total[$__rate_interval]))`, "Committed (success={{success}})")).
			Target(promQ("C", `rate(avalanche_evm_firewood_firewood_proposals_discarded_total[$__rate_interval])`, "Discarded")).
			Target(promQ("D", `rate(avalanche_evm_firewood_firewood_proposals_reparented_total[$__rate_interval])`, "Reparented"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-47", v2.NewPanelBuilder().Id(47).
		Title("Firewood: Commit Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_firewood_commits_total[$__rate_interval])`, "Commits/s")).
			Target(promQ("B", `rate(avalanche_evm_firewood_firewood_commits_blocked_total[$__rate_interval])`, "Blocked/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-46", v2.NewPanelBuilder().Id(46).
		Title("Firewood: Revision Status").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_firewood_revisions_active`, "Active Revisions")).
			Target(promQ("B", `avalanche_evm_firewood_firewood_revisions_limit`, "Revision Limit")).
			Target(promQ("C", `avalanche_evm_firewood_firewood_proposals_uncommitted`, "Uncommitted Proposals")).
			Target(promQ("D", `avalanche_evm_firewood_firewood_nodes_pending_deletion`, "Nodes Pending Deletion"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-48", v2.NewPanelBuilder().Id(48).
		Title("Firewood: Trie Operations").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (operation) (rate(avalanche_evm_firewood_firewood_node_inserts_total[$__rate_interval]))`, "Inserts ({{operation}})")).
			Target(promQ("B", `sum by (result) (rate(avalanche_evm_firewood_firewood_node_removes_total[$__rate_interval]))`, "Removes ({{result}})")).
			Target(promQ("C", `rate(avalanche_evm_firewood_firewood_change_proof_iterations_total[$__rate_interval])`, "Change Proof Iterations"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-67", v2.NewPanelBuilder().Id(67).
		Title("Firewood: Persist Permits").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_firewood_persist_permits_available`, "Available")).
			Target(promQ("B", `avalanche_evm_firewood_firewood_persist_permits_limit`, "Limit"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-63", v2.NewPanelBuilder().Id(63).
		Title("Firewood: Root Store").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (success) (rate(avalanche_evm_firewood_firewood_persist_root_store_total[$__rate_interval]))`, "Persist/s (success={{success}})")).
			Target(promQ("B", `rate(avalanche_evm_firewood_firewood_rootstore_lookups_total[$__rate_interval])`, "Lookups/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildWritePathRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Write Path").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-44", 0, 0, 12, 8)).
			Item(gridItem("panel-47", 12, 0, 12, 8)).
			Item(gridItem("panel-46", 0, 8, 12, 8)).
			Item(gridItem("panel-48", 12, 8, 12, 8)).
			Item(gridItem("panel-67", 0, 16, 12, 8)).
			Item(gridItem("panel-63", 12, 16, 12, 8)))
}

// ── Read Path row ────────────────────────────────────────────────────────────

func (c *CChain) registerReadPathPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-60", v2.NewPanelBuilder().Id(60).
		Title("Firewood: Node Read Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum by (from) (rate(avalanche_evm_firewood_firewood_node_reads_total[$__rate_interval]))`, "{{from}}"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-53", v2.NewPanelBuilder().Id(53).
		Title("Firewood: Node Cache Hit Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum(rate(avalanche_evm_firewood_firewood_node_cache_accesses_total{type="hit"}[$__rate_interval])) / sum(rate(avalanche_evm_firewood_firewood_node_cache_accesses_total[$__rate_interval]))`, "Node Cache")).
			Target(promQ("B", `sum(rate(avalanche_evm_firewood_firewood_freelist_cache_accesses_total{type="hit"}[$__rate_interval])) / sum(rate(avalanche_evm_firewood_firewood_freelist_cache_accesses_total[$__rate_interval]))`, "Freelist Cache"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("percentunit").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-54", v2.NewPanelBuilder().Id(54).
		Title("Firewood: Node Cache Memory").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_firewood_node_cache_bytes`, "Used")).
			Target(promQ("B", `avalanche_evm_firewood_firewood_node_cache_limit_bytes`, "Limit"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("bytes").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-62", v2.NewPanelBuilder().Id(62).
		Title("Firewood: Free List State").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum(avalanche_evm_firewood_firewood_free_list_entries)`, "Free List Entries")).
			Target(promQ("B", `avalanche_evm_firewood_firewood_freelist_cache_entries`, "Free List Cache Entries"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildReadPathRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Read Path").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-60", 0, 0, 12, 8)).
			Item(gridItem("panel-53", 12, 0, 12, 8)).
			Item(gridItem("panel-54", 0, 8, 12, 8)).
			Item(gridItem("panel-62", 12, 8, 12, 8)))
}

// ── Latency Distributions row ────────────────────────────────────────────────

func (c *CChain) registerLatencyPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-45", v2.NewPanelBuilder().Id(45).
		Title("Firewood: Proposal Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_proposal_commit_duration_seconds_bucket[$__rate_interval])) by (le, success))`, "Commit p50 (success={{success}})")).
			Target(promQ("B", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_proposal_commit_duration_seconds_bucket[$__rate_interval])) by (le, success))`, "Commit p99 (success={{success}})")).
			Target(promQ("C", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_propose_duration_seconds_bucket[$__rate_interval])) by (le))`, "Propose p50")).
			Target(promQ("D", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_propose_duration_seconds_bucket[$__rate_interval])) by (le))`, "Propose p99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-49", v2.NewPanelBuilder().Id(49).
		Title("Firewood: Persist Worker Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_persist_cycle_duration_seconds_bucket[$__rate_interval])) by (le))`, "Cycle p50")).
			Target(promQ("B", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_persist_cycle_duration_seconds_bucket[$__rate_interval])) by (le))`, "Cycle p99")).
			Target(promQ("C", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_flush_duration_seconds_bucket[$__rate_interval])) by (le))`, "Flush p50")).
			Target(promQ("D", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_flush_duration_seconds_bucket[$__rate_interval])) by (le))`, "Flush p99")).
			Target(promQ("E", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_reap_duration_seconds_bucket[$__rate_interval])) by (le))`, "Reap p50")).
			Target(promQ("F", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_reap_duration_seconds_bucket[$__rate_interval])) by (le))`, "Reap p99")).
			Target(promQ("G", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_persist_submit_duration_seconds[$__rate_interval])))`, "Submit p99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-56", v2.NewPanelBuilder().Id(56).
		Title("Firewood: I/O Read Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_io_read_duration_seconds[$__rate_interval])))`, "p50")).
			Target(promQ("B", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_io_read_duration_seconds[$__rate_interval])))`, "p90")).
			Target(promQ("C", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_io_read_duration_seconds[$__rate_interval])))`, "p99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-64", v2.NewPanelBuilder().Id(64).
		Title("Firewood: Persist Root Store Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_persist_root_store_duration_seconds_bucket[$__rate_interval])) by (le, success))`, "p50 ({{success}})")).
			Target(promQ("B", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_persist_root_store_duration_seconds_bucket[$__rate_interval])) by (le, success))`, "p99 ({{success}})"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-50", v2.NewPanelBuilder().Id(50).
		Title("Firewood: Lock Contention").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.9, sum(rate(avalanche_evm_firewood_firewood_commit_lock_wait_seconds[$__rate_interval])))`, "Commit Lock p90")).
			Target(promQ("B", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_commit_lock_wait_seconds[$__rate_interval])))`, "Commit Lock p99")).
			Target(promQ("C", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_current_revision_lock_wait_seconds[$__rate_interval])))`, "Current Rev Lock p90")).
			Target(promQ("D", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_current_revision_lock_wait_seconds[$__rate_interval])))`, "Current Rev Lock p99")).
			Target(promQ("E", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_by_hash_lock_wait_seconds[$__rate_interval])))`, "By Hash Lock p90")).
			Target(promQ("F", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_by_hash_lock_wait_seconds[$__rate_interval])))`, "By Hash Lock p99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-58", v2.NewPanelBuilder().Id(58).
		Title("Firewood: Replay Latency").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_replay_propose_duration_seconds_bucket[$__rate_interval])) by (le))`, "Propose p50")).
			Target(promQ("B", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_replay_propose_duration_seconds_bucket[$__rate_interval])) by (le))`, "Propose p99")).
			Target(promQ("C", `histogram_quantile(0.50, sum(rate(avalanche_evm_firewood_firewood_replay_commit_duration_seconds_bucket[$__rate_interval])) by (le))`, "Commit p50")).
			Target(promQ("D", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_replay_commit_duration_seconds_bucket[$__rate_interval])) by (le))`, "Commit p99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildLatencyRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Latency Distributions").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-45", 0, 0, 12, 8)).
			Item(gridItem("panel-49", 12, 0, 12, 8)).
			Item(gridItem("panel-56", 0, 8, 12, 8)).
			Item(gridItem("panel-64", 12, 8, 12, 8)).
			Item(gridItem("panel-50", 0, 16, 12, 8)).
			Item(gridItem("panel-58", 12, 16, 12, 8)))
}

// ── Storage & I/O row ────────────────────────────────────────────────────────

func (c *CChain) registerStorageIOPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-51", v2.NewPanelBuilder().Id(51).
		Title("Firewood: Storage Allocation").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum(rate(avalanche_evm_firewood_firewood_storage_bytes_appended_total[$__rate_interval]))`, "Appended")).
			Target(promQ("B", `sum(rate(avalanche_evm_firewood_firewood_storage_bytes_reused_total[$__rate_interval]))`, "Reused")).
			Target(promQ("C", `sum(rate(avalanche_evm_firewood_firewood_storage_bytes_freed_total[$__rate_interval]))`, "Freed")).
			Target(promQ("D", `sum(rate(avalanche_evm_firewood_firewood_storage_bytes_wasted_total[$__rate_interval]))`, "Wasted (fragmentation)"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("Bps").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-52", v2.NewPanelBuilder().Id(52).
		Title("Firewood: Database Size").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_firewood_database_size_bytes`, "Database Size"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("bytes").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-61", v2.NewPanelBuilder().Id(61).
		Title("Firewood: Node Allocation Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `sum(rate(avalanche_evm_firewood_firewood_nodes_allocated_total[$__rate_interval]))`, "Allocated/s")).
			Target(promQ("B", `sum(rate(avalanche_evm_firewood_firewood_nodes_deleted_total[$__rate_interval]))`, "Deleted/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-55", v2.NewPanelBuilder().Id(55).
		Title("Firewood: Disk I/O Byte Throughput").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_firewood_io_bytes_read_total[$__rate_interval])`, "Bytes Read/s")).
			Target(promQ("B", `rate(avalanche_evm_firewood_firewood_io_bytes_written_total[$__rate_interval])`, "Bytes Written/s"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("Bps").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-59", v2.NewPanelBuilder().Id(59).
		Title("Firewood: Disk I/O Operation Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_firewood_io_reads_total[$__rate_interval])`, "Read Ops/s")).
			Target(promQ("B", `rate(avalanche_evm_firewood_firewood_io_writes_total[$__rate_interval])`, "Write Ops/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-66", v2.NewPanelBuilder().Id(66).
		Title("Firewood: io_uring Diagnostics").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_firewood_io_uring_ring_full_total[$__rate_interval])`, "Ring Full/s")).
			Target(promQ("B", `rate(avalanche_evm_firewood_firewood_io_uring_eagain_retries_total[$__rate_interval])`, "EAGAIN Retries/s")).
			Target(promQ("C", `rate(avalanche_evm_firewood_firewood_io_uring_partial_write_retries_total[$__rate_interval])`, "Partial Write Retries/s")).
			Target(promQ("D", `rate(avalanche_evm_firewood_firewood_io_uring_sq_waits_total[$__rate_interval])`, "SQ Waits/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildStorageIORow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Storage & I/O").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-51", 0, 0, 12, 8)).
			Item(gridItem("panel-52", 12, 0, 12, 8)).
			Item(gridItem("panel-61", 0, 8, 12, 8)).
			Item(gridItem("panel-55", 12, 8, 12, 8)).
			Item(gridItem("panel-59", 0, 16, 12, 8)).
			Item(gridItem("panel-66", 12, 16, 12, 8)))
}

// ── FFI & Proof row ──────────────────────────────────────────────────────────

func (c *CChain) registerFFIProofPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-43", v2.NewPanelBuilder().Id(43).
		Title("Firewood: Metrics Gather Time").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_avg(rate(avalanche_evm_firewood_firewood_gather_duration_seconds[$__rate_interval]))`, "Gather Duration Avg")).
			Target(promQ("B", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_gather_duration_seconds[$__rate_interval])))`, "Gather Duration P90")).
			Target(promQ("C", `histogram_quantile(0.99, sum(rate(avalanche_evm_firewood_firewood_gather_duration_seconds[$__rate_interval])))`, "Gather Duration P99"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-65", v2.NewPanelBuilder().Id(65).
		Title("Firewood: Proof Merge Rate").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `rate(avalanche_evm_firewood_firewood_proof_merges_total[$__rate_interval])`, "Proof Merges/s"))).
		Visualization(timeseries.NewVisualizationBuilder().
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-57", v2.NewPanelBuilder().Id(57).
		Title("Firewood: Proof Serialization").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_go_proof_marshal_duration_seconds_bucket[$__rate_interval])) by (le, proof_type))`, "Marshal p90 ({{proof_type}})")).
			Target(promQ("B", `histogram_quantile(0.90, sum(rate(avalanche_evm_firewood_firewood_go_proof_unmarshal_duration_seconds_bucket[$__rate_interval])) by (le, proof_type))`, "Unmarshal p90 ({{proof_type}})"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("s").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildFFIProofRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("FFI & Proof").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-43", 0, 0, 12, 8)).
			Item(gridItem("panel-65", 12, 0, 12, 8)).
			Item(gridItem("panel-57", 0, 8, 24, 8)))
}

// ── Memory row ───────────────────────────────────────────────────────────────

func (c *CChain) registerMemoryPanels(b *v2.DashboardBuilder) {
	b.Panel("panel-68", v2.NewPanelBuilder().Id(68).
		Title("Firewood: jemalloc Memory").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_jemalloc_allocated_bytes`, "Allocated")).
			Target(promQ("B", `avalanche_evm_firewood_jemalloc_active_bytes`, "Active")).
			Target(promQ("C", `avalanche_evm_firewood_jemalloc_resident_bytes`, "Resident")).
			Target(promQ("D", `avalanche_evm_firewood_jemalloc_mapped_bytes`, "Mapped"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("bytes").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))

	b.Panel("panel-69", v2.NewPanelBuilder().Id(69).
		Title("Firewood: jemalloc Overhead").
		Data(v2.NewQueryGroupBuilder().
			Target(promQ("A", `avalanche_evm_firewood_jemalloc_metadata_bytes`, "Metadata")).
			Target(promQ("B", `avalanche_evm_firewood_jemalloc_retained_bytes`, "Retained"))).
		Visualization(timeseries.NewVisualizationBuilder().Unit("bytes").
			Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeSingle, common.SortOrderNone))))
}

func (c *CChain) buildMemoryRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Memory").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-68", 0, 0, 12, 8)).
			Item(gridItem("panel-69", 12, 0, 12, 8)))
}

// ── Space Allocation & Free Lists row ────────────────────────────────────────

func (c *CChain) registerSpaceAllocationPanels(b *v2.DashboardBuilder) {
	byIndex := func(key, metric, unit string, fill float64, stacked bool) *v2.PanelBuilder {
		vis := timeseries.NewVisualizationBuilder().Unit(unit)
		if fill > 0 {
			vis.FillOpacity(fill)
		}
		if stacked {
			vis.Stacking(stackingPercent())
		}
		vis.Legend(legendOpts(nil, common.LegendDisplayModeList)).
			Tooltip(tooltipOpts(common.TooltipDisplayModeMulti, common.SortOrderNone))
		return v2.NewPanelBuilder().
			Title(key).
			Data(v2.NewQueryGroupBuilder().
				Target(promQ("A", metric, "{{index}}"))).
			Visualization(vis)
	}

	rateByIndex := func(metric string) string {
		return fmt.Sprintf("sum by (index) (rate(%s[$__rate_interval]))", metric)
	}

	b.Panel("panel-70", byIndex("Firewood: Storage Bytes Reused by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_reused_total"), "percent", 80, true).Id(70))
	b.Panel("panel-71", byIndex("Firewood: Storage Bytes Reused by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_reused_total"), "Bps", 0, false).Id(71))

	b.Panel("panel-72", byIndex("Firewood: Storage Bytes Appended by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_appended_total"), "percent", 80, true).Id(72))
	b.Panel("panel-73", byIndex("Firewood: Storage Bytes Appended by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_appended_total"), "Bps", 0, false).Id(73))

	b.Panel("panel-74", byIndex("Firewood: Storage Bytes Freed by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_freed_total"), "percent", 80, true).Id(74))
	b.Panel("panel-75", byIndex("Firewood: Storage Bytes Freed by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_freed_total"), "Bps", 0, false).Id(75))

	b.Panel("panel-76", byIndex("Firewood: Storage Bytes Wasted by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_wasted_total"), "percent", 80, true).Id(76))
	b.Panel("panel-77", byIndex("Firewood: Storage Bytes Wasted by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_storage_bytes_wasted_total"), "Bps", 0, false).Id(77))

	b.Panel("panel-78", byIndex("Firewood: Nodes Allocated by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_nodes_allocated_total"), "percent", 80, true).Id(78))
	b.Panel("panel-79", byIndex("Firewood: Nodes Allocated by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_nodes_allocated_total"), "short", 0, false).Id(79))

	b.Panel("panel-80", byIndex("Firewood: Nodes Deleted by Index (Stacked)",
		rateByIndex("avalanche_evm_firewood_firewood_nodes_deleted_total"), "percent", 80, true).Id(80))
	b.Panel("panel-81", byIndex("Firewood: Nodes Deleted by Index (Lines)",
		rateByIndex("avalanche_evm_firewood_firewood_nodes_deleted_total"), "short", 0, false).Id(81))

	b.Panel("panel-82", byIndex("Firewood: Free List Entries by Index (Stacked)",
		"sum by (index) (avalanche_evm_firewood_firewood_free_list_entries)", "percent", 80, true).Id(82))
	b.Panel("panel-83", byIndex("Firewood: Free List Entries by Index (Lines)",
		"sum by (index) (avalanche_evm_firewood_firewood_free_list_entries)", "short", 0, false).Id(83))
}

func (c *CChain) buildSpaceAllocationRow() *v2.RowBuilder {
	return v2.NewRowBuilder().Title("Space Allocation & Free Lists").
		GridLayout(v2.NewGridBuilder().
			Item(gridItem("panel-70", 0, 0, 12, 8)).
			Item(gridItem("panel-71", 12, 0, 12, 8)).
			Item(gridItem("panel-72", 0, 8, 12, 8)).
			Item(gridItem("panel-73", 12, 8, 12, 8)).
			Item(gridItem("panel-74", 0, 16, 12, 8)).
			Item(gridItem("panel-75", 12, 16, 12, 8)).
			Item(gridItem("panel-76", 0, 24, 12, 8)).
			Item(gridItem("panel-77", 12, 24, 12, 8)).
			Item(gridItem("panel-78", 0, 32, 12, 8)).
			Item(gridItem("panel-79", 12, 32, 12, 8)).
			Item(gridItem("panel-80", 0, 40, 12, 8)).
			Item(gridItem("panel-81", 12, 40, 12, 8)).
			Item(gridItem("panel-82", 0, 48, 12, 8)).
			Item(gridItem("panel-83", 12, 48, 12, 8)))
}
