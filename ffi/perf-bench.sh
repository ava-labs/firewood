# should be only needed for kernel stack
# sudo sysctl kernel.perf_event_paranoid=-1
# sudo sysctl kernel.kptr_restrict=0

set -euo pipefail

# Parameters (override via env):
# BENCH_RE: regex for go test -bench
# BENCHTIME: eg 1x for one full scan; 200ms for timed runs
# PERF_FREQ: perf sampling frequency
# PERF_CALLGRAPH: call graph mode (dwarf,16384 or fp)
# USE_PERF: if set to 1, run perf+flamegraph; otherwise only pprof
BENCH_RE=${BENCH_RE:-"BenchmarkIterator(\/(Next.*|Exhaust.*)|Matrix|Exhaust)"}
# BENCH_RE=${BENCH_RE:-"BenchmarkIterator(\/(Exhaust.*)|Exhaust)"}
BENCHTIME=${BENCHTIME:-"1x"}
PERF_FREQ=${PERF_FREQ:-200}
PERF_CALLGRAPH=${PERF_CALLGRAPH:-"dwarf,16384"}
USE_PERF=${USE_PERF:-0}

# Rust passes proper parameters but this could be used without changing the profiles
RUSTFLAGS="-C debuginfo=2 -C force-frame-pointers=yes -C link-arg=-Wl" cargo build --release

mkdir -p target/perf/

# Build a test binary; for perf stacks, disabling inlining helps readability.
GOEXPERIMENT=cgocheck2 go test -c -o target/bench.test -gcflags=all="-N -l"

# Optional: Go profiles for Go-side flamegraphs and heap insights
# Also capture verbose output for plotting runtime metrics (rt: ...) and bench lines.

# GODEBUG=${GODEBUG:-""} \
RUST_BACKTRACE=1 \
GODEBUG=cgocheck=1,invalidptr=1 \
GOTRACEBACK=crash \
./target/bench.test \
  -test.run=^$ -test.v -test.bench="$BENCH_RE" \
  -test.benchmem -test.benchtime=$BENCHTIME \
  -test.cpuprofile=target/perf/cpu.pprof \
  -test.memprofile=target/perf/mem.pprof | tee target/perf/bench.txt || true

if [ "$USE_PERF" = "1" ]; then
  echo "Running Linux perf sampling..."
  perf record -F "$PERF_FREQ" -o target/perf/perf.data \
    --call-graph "$PERF_CALLGRAPH" \
    -e cycles:u -- \
    ./target/bench.test \
    -test.run=^$ -test.bench="$BENCH_RE" \
    -test.benchmem -test.benchtime=$BENCHTIME || true

  perf script -i target/perf/perf.data > target/perf/out.perf
  if [ -x ../../FlameGraph/stackcollapse-perf.pl ] && [ -x ../../FlameGraph/flamegraph.pl ]; then
    ../../FlameGraph/stackcollapse-perf.pl target/perf/out.perf > target/perf/out.folded
    ../../FlameGraph/flamegraph.pl target/perf/out.folded > target/perf/flame.svg
    echo "FlameGraph at target/perf/flame.svg"
  else
    echo "FlameGraph scripts not found at ../../FlameGraph; skipping SVG generation"
  fi
fi

echo "Go pprof profiles written under target/perf/"
