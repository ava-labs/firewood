// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/stretchr/testify/require"

	dto "github.com/prometheus/client_model/go"
)

var (
	expectedMetrics = map[string]dto.MetricType{
		"firewood_proposal_commits_total":         dto.MetricType_COUNTER,
		"firewood_flush_duration_seconds":         dto.MetricType_HISTOGRAM,
		"firewood_persist_cycle_duration_seconds": dto.MetricType_HISTOGRAM,
		"firewood_node_inserts_total":             dto.MetricType_COUNTER,
		"firewood_storage_bytes_appended_total":   dto.MetricType_COUNTER,
		// jemalloc memory allocator gauges (bytes).
		// jemalloc_retained_bytes is omitted because it can legitimately be zero
		// on some platforms, and we assert that gauge values are positive below.
		"jemalloc_active_bytes":    dto.MetricType_GAUGE,
		"jemalloc_allocated_bytes": dto.MetricType_GAUGE,
		"jemalloc_metadata_bytes":  dto.MetricType_GAUGE,
		"jemalloc_mapped_bytes":    dto.MetricType_GAUGE,
		"jemalloc_resident_bytes":  dto.MetricType_GAUGE,
	}
	initMetrics   sync.Once
	initLogs      sync.Once
	activeLogPath string
)

func ensureMetricsStarted(t *testing.T) {
	t.Helper()
	initMetrics.Do(func() {
		require.NoError(t, StartMetrics())
	})
}

func ensureLogsStarted(t *testing.T) {
	t.Helper()
	initLogs.Do(func() {
		// The global logger writes here for the rest of the process, so the
		// file must outlive whichever test happens to initialize it first;
		// t.TempDir() would be deleted when that test finishes.
		logDir, err := os.MkdirTemp("", "firewood-logs") //nolint:usetesting // must outlive the initializing test
		require.NoError(t, err)
		logPath := filepath.Join(logDir, "firewood.log")
		logConfig := &LogConfig{
			Path:        logPath,
			FilterLevel: "trace",
		}
		if err := StartLogs(logConfig); err != nil {
			// "Logging is not available" error occurs when the FFI library was
			// built without the "logger" feature flag enabled.
			require.ErrorContains(t, err, "Logging is not available")
			activeLogPath = ""
			return
		}
		activeLogPath = logPath
	})
}

func newDbWithMetricsAndLogs(t *testing.T, opts ...Option) (db *Database, logPath string) {
	t.Helper()
	db = newTestDatabase(t, opts...)
	ensureMetricsStarted(t)
	ensureLogsStarted(t)
	return db, activeLogPath
}

// TestMetrics verifies that expected metrics are populated after a database
// operation. This lives under one test as we can only instantiate the global
// recorder once.
func TestMetrics(t *testing.T) {
	r := require.New(t)

	const dbTag = "metrics-test-db"
	db, logPath := newDbWithMetricsAndLogs(t, WithMetricsTag(dbTag))
	// batch update
	_, _, batch := kvForTest(10)
	_, err := db.Update(batch)
	r.NoError(err)

	// Close database to ensure background persistence completes before checking metrics.
	// The flush_nodes metric is recorded during persistence, which happens asynchronously.
	r.NoError(db.Close(t.Context()))

	families, err := GatherRenderedMetrics()
	r.NoError(err)
	r.NotEmpty(families)

	byName := make(map[string]*dto.MetricFamily, len(families))
	for _, mf := range families {
		byName[mf.GetName()] = mf
	}

	for name, wantType := range expectedMetrics {
		mf, ok := byName[name]
		r.True(ok, "metric %q not found", name)
		r.Equal(wantType, mf.GetType(), "metric %q has wrong type", name)

		// Jemalloc gauges must report positive byte counts in a running process.
		if wantType == dto.MetricType_GAUGE && len(mf.Metric) > 0 && mf.Metric[0].Gauge != nil {
			r.Greater(*mf.Metric[0].Gauge.Value, 0.0, "metric %q should be positive", name)
		}
	}

	if logPath != "" {
		r.True(assertNonEmptyFile(t, logPath))
		logContents, readErr := os.ReadFile(logPath)
		r.NoError(readErr)
		r.Contains(string(logContents), "db_tag="+dbTag)
	}
}

// gatherForTag gathers all metrics and returns only those carrying a db_tag
// label equal to tag, the way a consumer of [Gatherer] would filter them.
func gatherForTag(tag string) ([]*dto.MetricFamily, error) {
	families, err := (Gatherer{}).Gather()
	if err != nil {
		return nil, err
	}

	filtered := make([]*dto.MetricFamily, 0, len(families))
	for _, mf := range families {
		var metrics []*dto.Metric
		for _, metric := range mf.GetMetric() {
			for _, pair := range metric.GetLabel() {
				if pair.GetName() == "db_tag" && pair.GetValue() == tag {
					metrics = append(metrics, metric)
					break
				}
			}
		}
		if len(metrics) > 0 {
			filtered = append(filtered, &dto.MetricFamily{
				Name:   mf.Name,
				Help:   mf.Help,
				Type:   mf.Type,
				Unit:   mf.Unit,
				Metric: metrics,
			})
		}
	}
	return filtered, nil
}

// assertAllTagged asserts every metric in families carries a db_tag label equal to tag.
func assertAllTagged(r *require.Assertions, families []*dto.MetricFamily, tag string) {
	r.NotEmpty(families)
	for _, family := range families {
		for _, metric := range family.GetMetric() {
			labels := make(map[string]string, len(metric.GetLabel()))
			for _, pair := range metric.GetLabel() {
				labels[pair.GetName()] = pair.GetValue()
			}
			r.Equal(tag, labels["db_tag"], "family %s", family.GetName())
		}
	}
}

func TestMetricsFilteredByDBTag(t *testing.T) {
	r := require.New(t)
	ensureMetricsStarted(t)

	tags := []string{"filter_by_db_tag_a", "filter_by_db_tag_b"}
	for _, tag := range tags {
		db := newTestDatabase(t, WithMetricsTag(tag))
		_, _, batch := kvForTest(3)
		_, err := db.Update(batch)
		r.NoError(err)
		r.NoError(db.Close(t.Context()))
	}

	for _, tag := range tags {
		families, err := gatherForTag(tag)
		r.NoError(err)
		assertAllTagged(r, families, tag)
	}
}

func TestMetricsFilterExcludesUntaggedDatabase(t *testing.T) {
	r := require.New(t)
	ensureMetricsStarted(t)
	const tag = "untagged_filter_test"

	// Expensive metrics are enabled so the expensive-gated commit duration
	// histogram is recorded and can be checked for the tag.
	dbTagged := newTestDatabase(t, WithMetricsTag(tag), WithExpensiveMetrics())
	_, _, batch := kvForTest(2)
	_, err := dbTagged.Update(batch)
	r.NoError(err)
	r.NoError(dbTagged.Close(t.Context()))

	dbUntagged := newTestDatabase(t)
	_, _, batch = kvForTest(2)
	_, err = dbUntagged.Update(batch)
	r.NoError(err)
	r.NoError(dbUntagged.Close(t.Context()))

	// Filtering by tag must exclude the untagged database's series.
	families, err := gatherForTag(tag)
	r.NoError(err)
	assertAllTagged(r, families, tag)

	names := make(map[string]bool, len(families))
	for _, family := range families {
		names[family.GetName()] = true
	}
	r.True(names["firewood_proposal_commits_total"])
	r.True(names["firewood_proposal_commit_duration_seconds"])
}

// TestTagMetricsConcurrentTLSIsolation commits to several tagged databases
// concurrently and checks each tag's commit counter counts only its own commits.
func TestTagMetricsConcurrentTLSIsolation(t *testing.T) {
	r := require.New(t)
	ensureMetricsStarted(t)
	ensureLogsStarted(t)

	const workers = 4
	const commits = 5
	const counterName = "firewood_proposal_commits_total"

	tags := make([]string, workers)
	dbs := make([]*Database, workers)
	before := make([]float64, workers)
	for i := range workers {
		tags[i] = fmt.Sprintf("tls_isolation_worker_%d", i)
		dbs[i] = newTestDatabase(t, WithMetricsTag(tags[i]))
		v, err := gatherTaggedCounterValue(counterName, tags[i])
		r.NoError(err)
		before[i] = v
	}

	var wg sync.WaitGroup
	errs := make(chan error, workers)
	for i := range workers {
		wg.Go(func() {
			for c := range commits {
				key := fmt.Appendf(nil, "key_%d", c)
				if _, err := dbs[i].Update([]BatchOp{Put(key, []byte("value"))}); err != nil {
					errs <- fmt.Errorf("%s update %d: %w", tags[i], c, err)
					return
				}
			}
		})
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		r.NoError(err)
	}

	for i := range workers {
		r.NoError(dbs[i].Close(t.Context()))
		after, err := gatherTaggedCounterValue(counterName, tags[i])
		r.NoError(err)
		r.InDelta(commits, after-before[i], 1e-9, "db_tag=%s", tags[i])
	}

	// logs concurrency test
	// operations were byte-identical, so log counts should match
	if activeLogPath != "" {
		logContents, err := os.ReadFile(activeLogPath)
		r.NoError(err)
		s := string(logContents)
		// prevent prefix matches
		count0 := strings.Count(s, "db_tag="+tags[0]+"]")
		r.Positive(count0)
		for _, tag := range tags[1:] {
			r.Equal(count0, strings.Count(s, "db_tag="+tag+"]"), "db_tag=%s", tag)
		}
	}
}

func gatherTaggedCounterValue(metricName, dbTag string) (float64, error) {
	families, err := gatherForTag(dbTag)
	if err != nil {
		return 0, err
	}

	total := 0.0
	for _, family := range families {
		if family.GetName() != metricName {
			continue
		}

		if family.GetType() != dto.MetricType_COUNTER {
			return 0, fmt.Errorf("metric %q is not a counter", metricName)
		}

		for _, metric := range family.GetMetric() {
			counter := metric.GetCounter()
			if counter == nil {
				return 0, fmt.Errorf("counter metric %q missing counter value", metricName)
			}
			total += counter.GetValue()
		}
	}

	return total, nil
}

func TestGatherRenderedMetrics(t *testing.T) {
	r := require.New(t)

	// Ensure the metrics recorder is initialized.
	ensureMetricsStarted(t)

	// Call gather multiple times so the histogram accumulates observations.
	const gatherCalls = 3
	var allFamilies []*dto.MetricFamily
	for range gatherCalls {
		families, err := GatherRenderedMetrics()
		r.NoError(err)
		r.NotEmpty(families)
		allFamilies = families
	}

	// Find the native histogram metric for gather duration.
	var histFamily *dto.MetricFamily
	for _, mf := range allFamilies {
		// prometheus metric names are normalized to lowercase with underscores
		if mf.GetName() == "firewood_gather_duration_seconds" {
			histFamily = mf
			break
		}
	}
	r.NotNil(histFamily, "firewood_gather_duration_seconds metric not found")
	r.Equal(dto.MetricType_HISTOGRAM, histFamily.GetType())
	r.NotEmpty(histFamily.GetMetric())

	hist := histFamily.GetMetric()[0].GetHistogram()
	r.NotNil(hist, "histogram field must be set")

	// We called gather at least gatherCalls times; each call records one observation.
	// The first call won't see itself, but subsequent calls see prior observations.
	r.GreaterOrEqual(hist.GetSampleCount(), uint64(gatherCalls-1),
		"expected at least %d observations", gatherCalls-1)
	r.Greater(hist.GetSampleSum(), 0.0, "sample sum should be positive")

	// Validate native histogram fields are populated.
	r.NotNil(hist.Schema, "native histogram schema must be set")
	r.NotNil(hist.ZeroThreshold, "native histogram zero_threshold must be set")
	r.NotEmpty(hist.GetPositiveSpan(), "native histogram should have positive spans")
	r.NotEmpty(hist.GetPositiveDelta(), "native histogram should have positive deltas")
}

func assertNonEmptyFile(t *testing.T, path string) bool {
	t.Helper()
	f, err := os.ReadFile(path)
	require.NoError(t, err)
	require.NotEmpty(t, f)
	return true
}
