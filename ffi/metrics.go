// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

//go:generate go run generate_cgo.go

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"strings"
	"unsafe"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/common/expfmt"

	dto "github.com/prometheus/client_model/go"
)

var _ prometheus.Gatherer = (*Gatherer)(nil)

type Gatherer struct{}

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

// Starts global recorder for metrics.
// This function only needs to be called once.
// An error is returned if this method is called a second time, or if it is
// called after StartMetricsWithExporter.
func StartMetrics() error {
	result := C.fwd_start_metrics()
	return errorFromValue(&result)
}

// Start global recorder for metrics along with an HTTP exporter.
// This function only needs to be called once.
// An error is returned if this method is called a second time, if it is
// called after StartMetrics, or if the exporter failed to start.
func StartMetricsWithExporter(metricsPort uint16) error {
	result := C.fwd_start_metrics_with_exporter(C.uint16_t(metricsPort))
	return errorFromValue(&result)
}

// Collect metrics from global recorder
// Returns an error if the global recorder is not initialized.
// This method must be called after StartMetrics or StartMetricsWithExporter
func GatherMetrics() (string, error) {
	result := C.fwd_gather()
	b, err := bytesFromValue(&result)
	if err != nil {
		return "", err
	}
	return string(b), nil
}

// LogConfig configures logs for this process.
type LogConfig struct {
	Path        string
	FilterLevel string
}

// Starts global logs.
// This function only needs to be called once.
// An error is returned if this method is called a second time.
func StartLogs(config *LogConfig) error {
	args := &C.struct_LogArgs{}
	if config.Path != "" {
		args.path = C.CString(config.Path)
		defer C.free(unsafe.Pointer(args.path))
	}
	if config.FilterLevel != "" {
		args.filter_level = C.CString(config.FilterLevel)
		defer C.free(unsafe.Pointer(args.filter_level))
	}
	result := C.fwd_start_logs(args)
	return errorFromValue(&result)
}
