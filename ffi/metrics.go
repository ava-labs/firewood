// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
// #cgo noescape fwd_gather_rendered
// #cgo nocallback fwd_gather_rendered
// #cgo noescape fwd_free_rendered_metrics
// #cgo nocallback fwd_free_rendered_metrics
// #cgo noescape fwd_start_metrics
// #cgo nocallback fwd_start_metrics
// #cgo noescape fwd_start_metrics_with_exporter
// #cgo nocallback fwd_start_metrics_with_exporter
// #cgo noescape fwd_gather
// #cgo nocallback fwd_gather
// #cgo noescape fwd_start_logs
// #cgo nocallback fwd_start_logs
import "C"

import (
	"fmt"
	"runtime"
	"strings"
	"unsafe"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/common/expfmt"
	"google.golang.org/protobuf/proto"

	dto "github.com/prometheus/client_model/go"
)

var _ prometheus.Gatherer = (*Gatherer)(nil)

type Gatherer struct{}

func (Gatherer) Gather() ([]*dto.MetricFamily, error) {
	return GatherRenderedMetrics()
}

// TextGatherer is a [prometheus.Gatherer] that collects metrics from the
// firewood library by parsing text-rendered metrics.
//
// Deprecated: Use [Gatherer] instead, which uses structured data and avoids
// text rendering and parsing overhead.
type TextGatherer struct{}

func (TextGatherer) Gather() ([]*dto.MetricFamily, error) {
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

// GatherRenderedMetrics collects structured metrics from the global recorder
// and returns them as prometheus protobuf metric families.
// Returns an error if the global recorder is not initialized.
// This method must be called after StartMetrics or StartMetricsWithExporter.
func GatherRenderedMetrics() ([]*dto.MetricFamily, error) {
	result := C.fwd_gather_rendered()
	switch result.tag {
	case C.RenderedMetricsResult_Ok:
		owned := *(*C.OwnedRenderedMetrics)(unsafe.Pointer(&result.anon0))
		families, err := convertRenderedMetrics(owned)
		if freeErr := getErrorFromVoidResult(C.fwd_free_rendered_metrics(owned)); freeErr != nil {
			if err != nil {
				return nil, fmt.Errorf("%w (free error: %w)", err, freeErr)
			}
			return nil, fmt.Errorf("%w: %w", errFreeingValue, freeErr)
		}
		return families, err
	case C.RenderedMetricsResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.RenderedMetricsResult tag: %d", result.tag)
	}
}

func convertRenderedMetrics(owned C.OwnedRenderedMetrics) ([]*dto.MetricFamily, error) {
	if owned.ptr == nil {
		return nil, nil
	}

	cFamilies := unsafe.Slice((*C.OwnedMetricFamily)(unsafe.Pointer(owned.ptr)), owned.len)
	families := make([]*dto.MetricFamily, len(cFamilies))
	for i := range cFamilies {
		families[i] = convertMetricFamily(&cFamilies[i])
	}
	return families, nil
}

// borrowString returns a borrowed Go string that references the data in the
// provided C.OwnedBytes. The respective calls to [proto.String] in the
// conversion functions ensure that the data is copied into Go-owned memory,
// therefore the borrowed string is only valid for the duration of the
// conversion and should not be used after the conversion functions return.
func borrowString(b C.OwnedBytes) string {
	if b.ptr == nil {
		return ""
	}
	return string(unsafe.Slice((*byte)(b.ptr), b.len))
}

func convertMetricFamily(c *C.OwnedMetricFamily) *dto.MetricFamily {
	name := borrowString(c.name)

	mf := &dto.MetricFamily{
		Name: proto.String(name),
	}

	if c.help.tag == C.Maybe_OwnedBytes_Some_OwnedBytes {
		help := borrowString(*(*C.OwnedBytes)(unsafe.Pointer(&c.help.anon0)))
		mf.Help = proto.String(help)
	}

	if c.metrics.ptr == nil {
		return mf
	}

	cMetrics := unsafe.Slice((*C.OwnedMetric)(unsafe.Pointer(c.metrics.ptr)), c.metrics.len)
	mf.Metric = make([]*dto.Metric, len(cMetrics))

	// Determine the metric type from the first metric's value tag.
	if len(cMetrics) > 0 {
		mf.Type = metricTypeFromTag(cMetrics[0].value.tag)
	}

	for i := range cMetrics {
		mf.Metric[i] = convertMetric(&cMetrics[i])
	}

	return mf
}

func metricTypeFromTag(tag C.OwnedMetricValue_Tag) *dto.MetricType {
	switch tag {
	case C.OwnedMetricValue_Counter:
		return dto.MetricType_COUNTER.Enum()
	case C.OwnedMetricValue_Gauge:
		return dto.MetricType_GAUGE.Enum()
	case C.OwnedMetricValue_Summary:
		return dto.MetricType_SUMMARY.Enum()
	case C.OwnedMetricValue_ClassicHistogram:
		return dto.MetricType_HISTOGRAM.Enum()
	case C.OwnedMetricValue_NativeHistogram:
		return dto.MetricType_HISTOGRAM.Enum()
	default:
		return dto.MetricType_UNTYPED.Enum()
	}
}

func convertMetric(c *C.OwnedMetric) *dto.Metric {
	m := &dto.Metric{}

	if c.labels.ptr != nil {
		cLabels := unsafe.Slice((*C.OwnedLabelPair)(unsafe.Pointer(c.labels.ptr)), c.labels.len)
		m.Label = make([]*dto.LabelPair, len(cLabels))
		for i := range cLabels {
			m.Label[i] = &dto.LabelPair{
				Name:  proto.String(borrowString(cLabels[i].label)),
				Value: proto.String(borrowString(cLabels[i].value)),
			}
		}
	}

	switch c.value.tag {
	case C.OwnedMetricValue_Counter:
		val := *(*C.uint64_t)(unsafe.Pointer(&c.value.anon0))
		m.Counter = &dto.Counter{Value: proto.Float64(float64(val))}
	case C.OwnedMetricValue_Gauge:
		val := *(*C.double)(unsafe.Pointer(&c.value.anon0))
		m.Gauge = &dto.Gauge{Value: proto.Float64(float64(val))}
	case C.OwnedMetricValue_Summary:
		s := (*C.OwnedSummary)(unsafe.Pointer(&c.value.anon0))
		m.Summary = convertSummary(s)
	case C.OwnedMetricValue_ClassicHistogram:
		h := (*C.OwnedClassicHistogram)(unsafe.Pointer(&c.value.anon0))
		m.Histogram = convertClassicHistogram(h)
	case C.OwnedMetricValue_NativeHistogram:
		h := (*C.OwnedNativeHistogram)(unsafe.Pointer(&c.value.anon0))
		m.Histogram = convertNativeHistogram(h)
	}

	return m
}

func convertSummary(c *C.OwnedSummary) *dto.Summary {
	s := &dto.Summary{
		SampleCount: proto.Uint64(uint64(c.sample_count)),
		SampleSum:   proto.Float64(float64(c.sample_sum)),
	}

	if c.quantiles.ptr != nil {
		cQuantiles := unsafe.Slice((*C.OwnedQuantile)(unsafe.Pointer(c.quantiles.ptr)), c.quantiles.len)
		s.Quantile = make([]*dto.Quantile, len(cQuantiles))
		for i := range cQuantiles {
			s.Quantile[i] = &dto.Quantile{
				Quantile: proto.Float64(float64(cQuantiles[i].quantile)),
				Value:    proto.Float64(float64(cQuantiles[i].value)),
			}
		}
	}

	return s
}

func convertClassicHistogram(c *C.OwnedClassicHistogram) *dto.Histogram {
	h := &dto.Histogram{
		SampleCount: proto.Uint64(uint64(c.sample_count)),
		SampleSum:   proto.Float64(float64(c.sample_sum)),
	}

	if c.buckets.ptr != nil {
		cBuckets := unsafe.Slice((*C.OwnedBucket)(unsafe.Pointer(c.buckets.ptr)), c.buckets.len)
		h.Bucket = make([]*dto.Bucket, len(cBuckets))
		for i := range cBuckets {
			h.Bucket[i] = &dto.Bucket{
				CumulativeCount: proto.Uint64(uint64(cBuckets[i].cumulative_count)),
				UpperBound:      proto.Float64(float64(cBuckets[i].upper_bound)),
			}
		}
	}

	return h
}

func convertNativeHistogram(c *C.OwnedNativeHistogram) *dto.Histogram {
	h := &dto.Histogram{
		SampleCount:   proto.Uint64(uint64(c.sample_count)),
		SampleSum:     proto.Float64(float64(c.sample_sum)),
		Schema:        proto.Int32(int32(c.schema)),
		ZeroThreshold: proto.Float64(float64(c.zero_threshold)),
		ZeroCount:     proto.Uint64(uint64(c.zero_count)),
	}

	if c.positive_spans.ptr != nil {
		cSpans := unsafe.Slice((*C.OwnedBucketSpan)(unsafe.Pointer(c.positive_spans.ptr)), c.positive_spans.len)
		h.PositiveSpan = make([]*dto.BucketSpan, len(cSpans))
		for i := range cSpans {
			h.PositiveSpan[i] = &dto.BucketSpan{
				Offset: proto.Int32(int32(cSpans[i].offset)),
				Length: proto.Uint32(uint32(cSpans[i].length)),
			}
		}
	}

	if c.positive_deltas.ptr != nil {
		cDeltas := unsafe.Slice((*C.int64_t)(unsafe.Pointer(c.positive_deltas.ptr)), c.positive_deltas.len)
		h.PositiveDelta = make([]int64, len(cDeltas))
		for i := range cDeltas {
			h.PositiveDelta[i] = int64(cDeltas[i])
		}
	}

	if c.negative_spans.ptr != nil {
		cSpans := unsafe.Slice((*C.OwnedBucketSpan)(unsafe.Pointer(c.negative_spans.ptr)), c.negative_spans.len)
		h.NegativeSpan = make([]*dto.BucketSpan, len(cSpans))
		for i := range cSpans {
			h.NegativeSpan[i] = &dto.BucketSpan{
				Offset: proto.Int32(int32(cSpans[i].offset)),
				Length: proto.Uint32(uint32(cSpans[i].length)),
			}
		}
	}

	if c.negative_deltas.ptr != nil {
		cDeltas := unsafe.Slice((*C.int64_t)(unsafe.Pointer(c.negative_deltas.ptr)), c.negative_deltas.len)
		h.NegativeDelta = make([]int64, len(cDeltas))
		for i := range cDeltas {
			h.NegativeDelta[i] = int64(cDeltas[i])
		}
	}

	return h
}

// StartMetrics starts the global recorder for metrics.
// This function only needs to be called once.
// An error is returned if this method is called a second time, or if it is
// called after StartMetricsWithExporter.
// This is best used in conjunction with the [Gatherer] type to collect metrics.
func StartMetrics() error {
	return getErrorFromVoidResult(C.fwd_start_metrics())
}

// StartMetricsWithExporter starts the global recorder for metrics along with an
// HTTP exporter.
// This function only needs to be called once.
// An error is returned if this method is called a second time, if it is
// called after StartMetrics, or if the exporter failed to start.
func StartMetricsWithExporter(metricsPort uint16) error {
	return getErrorFromVoidResult(C.fwd_start_metrics_with_exporter(C.uint16_t(metricsPort)))
}

// GatherMetrics collects metrics from global recorder.
// Returns an error if the global recorder is not initialized.
// This method must be called after StartMetrics or StartMetricsWithExporter
func GatherMetrics() (string, error) {
	bytes, err := getValueFromValueResult(C.fwd_gather())
	if err != nil {
		return "", err
	}

	return string(bytes), nil
}

// LogConfig configures logs for this process.
type LogConfig struct {
	Path        string
	FilterLevel string
}

// StartLogs initialized the log factor in the Rust library with the provided
// configuration.
// This function only needs to be called once.
// An error is returned if this method is called a second time.
func StartLogs(config *LogConfig) error {
	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.struct_LogArgs{
		path:         newBorrowedBytes([]byte(config.Path), &pinner),
		filter_level: newBorrowedBytes([]byte(config.FilterLevel), &pinner),
	}

	return getErrorFromVoidResult(C.fwd_start_logs(args))
}
