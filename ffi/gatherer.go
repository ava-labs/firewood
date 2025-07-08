package ffi

import (
	"strings"

	"github.com/prometheus/client_golang/prometheus"
	dto "github.com/prometheus/client_model/go"
	"github.com/prometheus/common/expfmt"
)

var _ prometheus.Gatherer = (*Gatherer)(nil)

type Gatherer struct{}

// Gather implements prometheus.Gatherer.
func (Gatherer) Gather() ([]*dto.MetricFamily, error) {
	metrics, err := GatherMetrics()
	if err != nil {
		return nil, err
	}

	reader := strings.NewReader(metrics)

	var parser expfmt.TextParser
	parsedMetrics, err := parser.TextToMetricFamilies(reader)
	if err != nil {
		return nil, err
	}

	lst := make([]*dto.MetricFamily, 0, len(parsedMetrics))
	for _, v := range parsedMetrics {
		lst = append(lst, v)
	}

	return lst, nil
}
