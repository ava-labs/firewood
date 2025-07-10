package ffi

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"testing"
	"time"

	dto "github.com/prometheus/client_model/go"
	"github.com/stretchr/testify/require"
)

// Test calling metrics exporter along with gathering metrics
// This lives under one test as we can only instantiate the global recorder once
func TestMetrics(t *testing.T) {
	r := require.New(t)
	ctx := context.Background()

	db := newTestDatabase(t)

	metricsPort := uint16(3000)
	r.NoError(StartMetricsWithExporter(metricsPort))

	// Populate DB
	keys, vals := kvForTest(10)
	_, err := db.Update(keys, vals)
	r.NoError(err)

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

	// Check that batch op was recorded
	r.Contains(string(body), "firewood_ffi_batch 1")

	g := Gatherer{}
	metricsFamily, err := g.Gather()
	r.NoError(err)

	expectedMetrics := map[string]dto.MetricType{
		"firewood_ffi_batch":          dto.MetricType_COUNTER,
		"firewood_proposal_commit":    dto.MetricType_COUNTER,
		"firewood_proposal_commit_ms": dto.MetricType_COUNTER,
		"firewood_ffi_propose_ms":     dto.MetricType_COUNTER,
		"firewood_ffi_commit_ms":      dto.MetricType_COUNTER,
		"firewood_ffi_batch_ms":       dto.MetricType_COUNTER,
		"firewood_flush_nodes":        dto.MetricType_COUNTER,
		"firewood_insert":             dto.MetricType_COUNTER,
		"firewood_space_from_end":     dto.MetricType_COUNTER,
	}

	for k, v := range expectedMetrics {
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
