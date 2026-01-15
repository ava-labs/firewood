// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"fmt"
	"io"
	"maps"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	dto "github.com/prometheus/client_model/go"
)

var (
	metricsPort     = uint16(3000)
	expectedMetrics = map[string]dto.MetricType{
		"ffi_batch":          dto.MetricType_COUNTER,
		"proposal_commit":    dto.MetricType_COUNTER,
		"proposal_commit_ms": dto.MetricType_COUNTER,
		"ffi_propose_ms":     dto.MetricType_COUNTER,
		"ffi_commit_ms":      dto.MetricType_COUNTER,
		"ffi_batch_ms":       dto.MetricType_COUNTER,
		"flush_nodes":        dto.MetricType_COUNTER,
		"insert":             dto.MetricType_COUNTER,
		"space_from_end":     dto.MetricType_COUNTER,
	}
	expectedExpensiveMetrics = map[string]dto.MetricType{
		"ffi_commit_ms_bucket":  dto.MetricType_HISTOGRAM,
		"ffi_propose_ms_bucket": dto.MetricType_HISTOGRAM,
		"ffi_batch_ms_bucket":   dto.MetricType_HISTOGRAM,
	}
	initMetrics   sync.Once
	initLogs      sync.Once
	activeLogPath string
)

func ensureMetricsStarted(r *require.Assertions) {
	initMetrics.Do(func() {
		r.NoError(StartMetricsWithExporter(metricsPort))
	})
}

func ensureLogsStarted(r *require.Assertions, logPath string) {
	initLogs.Do(func() {
		logConfig := &LogConfig{
			Path:        logPath,
			FilterLevel: "trace",
		}
		if err := StartLogs(logConfig); err != nil {
			r.Contains(err.Error(), "Logging is not available")
			activeLogPath = ""
			return
		}
		activeLogPath = logPath
	})
}

func newDbWithMetricsAndLogs(t *testing.T, opts ...Option) (db *Database, logPath string) {
	r := require.New(t)
	db = newTestDatabase(t, opts...)
	ensureMetricsStarted(r)
	ensureLogsStarted(r, filepath.Join(t.TempDir(), "firewood.log"))
	return db, activeLogPath
}

// Test calling metrics exporter along with gathering metrics
// This lives under one test as we can only instantiate the global recorder once
func TestMetrics(t *testing.T) {
	r := require.New(t)

	db, logPath := newDbWithMetricsAndLogs(t)
	// batch update
	keys, vals := kvForTest(10)
	_, err := db.Update(keys, vals)
	r.NoError(err)

	assertMetrics(t, metricsPort, expectedMetrics)
	r.True((logPath == "") || assertNonEmptyFile(r, logPath))
}

func TestExpensiveMetrics(t *testing.T) {
	r := require.New(t)
	db, _ := newDbWithMetricsAndLogs(t, WithExpensiveMetrics())
	// batch update
	keys, vals := kvForTest(10)
	_, err := db.Update(keys, vals)
	r.NoError(err)

	merged := make(map[string]dto.MetricType, len(expectedMetrics)+len(expectedExpensiveMetrics))
	maps.Copy(merged, expectedMetrics)
	maps.Copy(merged, expectedExpensiveMetrics)
	assertMetrics(t, metricsPort, merged)
}

func assertNonEmptyFile(r *require.Assertions, path string) bool {
	f, err := os.ReadFile(path)
	r.NoError(err)
	r.NotEmpty(f)
	return true
}

func assertMetrics(t *testing.T, metricsPort uint16, expected map[string]dto.MetricType) {
	r := require.New(t)
	ctx := t.Context()

	req, err := http.NewRequestWithContext(
		ctx,
		http.MethodGet,
		fmt.Sprintf("http://localhost:%d", metricsPort),
		nil,
	)
	r.NoError(err)

	client := &http.Client{Timeout: 10 * time.Second}
	resp, err := client.Do(req)
	r.NoError(err)

	body, err := io.ReadAll(resp.Body)
	r.NoError(err)
	r.NoError(resp.Body.Close())

	// Check that batch op was recorded (no prefix)
	r.NotContains(string(body), "ffi_batch 0")

	g := Gatherer{}
	metricsFamily, err := g.Gather()
	r.NoError(err)

	for k, v := range expected {
		var d *dto.MetricFamily
		for _, m := range metricsFamily {
			if *m.Name == k {
				d = m
			}
		}
		r.NotNil(d)
		r.Equal(v, *d.Type)
	}
}
