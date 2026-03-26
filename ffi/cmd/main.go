// check_native_histograms.go
// Usage: go run check_native_histograms.go <port>
// Example: go run check_native_histograms.go 57596

package main

import (
	"fmt"
	"io"
	"net/http"
	"os"

	dto "github.com/prometheus/client_model/go"
	"github.com/prometheus/common/expfmt"
)

const acceptHeader = "application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited"

func main() {
	port := "9090"
	if len(os.Args) > 1 {
		port = os.Args[1]
	}
	url := fmt.Sprintf("http://127.0.0.1:%s/ext/metrics", port)

	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error creating request: %v\n", err)
		os.Exit(1)
	}
	req.Header.Set("Accept", acceptHeader)

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error fetching metrics: %v\n", err)
		os.Exit(1)
	}
	defer resp.Body.Close()

	fmt.Printf("Content-Type: %s\n\n", resp.Header.Get("Content-Type"))

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error reading body: %v\n", err)
		os.Exit(1)
	}

	decoder := expfmt.NewDecoder(
		io.NopCloser(newByteReader(data)),
		expfmt.FmtProtoDelim,
	)

	var (
		totalFamilies int
		classicHisto  []string
		nativeHisto   []string
	)

	for {
		mf := &dto.MetricFamily{}
		if err := decoder.Decode(mf); err == io.EOF {
			break
		} else if err != nil {
			fmt.Fprintf(os.Stderr, "decode error: %v\n", err)
			os.Exit(1)
		}
		totalFamilies++

		for _, m := range mf.GetMetric() {
			if m.GetHistogram() != nil {
				h := m.GetHistogram()
				if h.GetSampleCountFloat() > 0 || h.GetSampleCount() > 0 {
					if len(h.GetPositiveSpan()) > 0 || len(h.GetNegativeSpan()) > 0 {
						nativeHisto = append(nativeHisto, mf.GetName())
					} else {
						classicHisto = append(classicHisto, mf.GetName())
					}
				}
			}
		}
	}

	fmt.Printf("Total metric families decoded: %d\n\n", totalFamilies)

	fmt.Printf("Classic histograms (%d):\n", len(classicHisto))
	for _, name := range classicHisto {
		fmt.Printf("  %s\n", name)
	}

	fmt.Printf("\nNative histograms (%d):\n", len(nativeHisto))
	if len(nativeHisto) == 0 {
		fmt.Println("  (none) — ffi.Gatherer is still using TextParser, native histograms are being dropped")
	}
	for _, name := range nativeHisto {
		fmt.Printf("  ✓ %s\n", name)
	}

	// Print native histogram details
	if len(nativeHisto) > 0 {
		fmt.Println("\nNative histogram details:")
		// re-decode to print details
		decoder2 := expfmt.NewDecoder(
			io.NopCloser(newByteReader(data)),
			expfmt.FmtProtoDelim,
		)
		for {
			mf := &dto.MetricFamily{}
			if err := decoder2.Decode(mf); err == io.EOF {
				break
			} else if err != nil {
				break
			}
			for _, m := range mf.GetMetric() {
				h := m.GetHistogram()
				if h == nil || len(h.GetPositiveSpan()) == 0 {
					continue
				}
				fmt.Printf("\n  %s\n", mf.GetName())
				fmt.Printf("    count:  %d\n", h.GetSampleCount())
				fmt.Printf("    sum:    %.6f\n", h.GetSampleSum())
				fmt.Printf("    schema: %d\n", h.GetSchema())
				fmt.Printf("    zero_count: %d\n", h.GetZeroCount())
				fmt.Printf("    zero_threshold: %g\n", h.GetZeroThreshold())
				fmt.Println("    positive spans:")
				for _, s := range h.GetPositiveSpan() {
					fmt.Printf("      offset=%d length=%d\n", s.GetOffset(), s.GetLength())
				}
				fmt.Println("    positive deltas:")
				for i, d := range h.GetPositiveDelta() {
					fmt.Printf("      [%d] %d\n", i, d)
				}
			}
		}
	}
}

type byteReader struct {
	data []byte
	pos  int
}

func newByteReader(data []byte) *byteReader { return &byteReader{data: data} }
func (r *byteReader) Read(p []byte) (int, error) {
	if r.pos >= len(r.data) {
		return 0, io.EOF
	}
	n := copy(p, r.data[r.pos:])
	r.pos += n
	return n, nil
}
func (r *byteReader) ReadByte() (byte, error) {
	if r.pos >= len(r.data) {
		return 0, io.EOF
	}
	b := r.data[r.pos]
	r.pos++
	return b, nil
}
