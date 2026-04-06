#!/bin/bash
# Run 8 workers (2 per spec) simulating BF=3, MaxDepth=4 for ~2 hours.
# Restarts workers that hit violations with new seeds.

set -e

HOURS="${1:-2}"
DURATION=$((HOURS * 3600))
JAVA=/opt/homebrew/opt/openjdk/bin/java
JAR="$(dirname "$0")/tla2tools.jar"
LOGDIR="/tmp/bf3md4_$(date +%s)"
COMBINED="$LOGDIR/all_violations.log"

mkdir -p "$LOGDIR"
cd "$(dirname "$0")"

echo "Running 8 workers for $HOURS hours: BF=3 MaxDepth=4 simulate"
echo "Logs in $LOGDIR"
echo ""

END_TIME=$(($(date +%s) + DURATION))
SEED_BASE=100

cat > "$LOGDIR/ctm.cfg" << 'EOF'
CONSTANTS
    BF = 3
    MaxDepth = 4
    NumValues = 2
    None = None
INIT SimInit
NEXT Next
INVARIANT
    RoundTrip
    LookupCorrect
    StructurallyValid
    DepthBounded
EOF

cat > "$LOGDIR/trio.cfg" << 'EOF'
CONSTANTS
    BF = 3
    MaxDepth = 4
    NumValues = 2
    None = None
INIT Init
NEXT Next
INVARIANT
    TrieMatchesShadow
    TrieWellFormed
    LookupMatchesShadow
EOF

cat > "$LOGDIR/cpv.cfg" << 'EOF'
CONSTANTS
    BF = 3
    MaxDepth = 4
    NumValues = 2
    None = None
INIT SimInit
NEXT Next
INVARIANT
    HonestProofAccepted
EOF

cat > "$LOGDIR/adv.cfg" << 'EOF'
CONSTANTS
    BF = 3
    MaxDepth = 4
    NumValues = 2
    None = None
INIT SimInit
NEXT Next
INVARIANT
    HonestProofAccepted
    OnlyCorrectDiffAccepted
EOF

worker_loop() {
    local wid=$1 spec=$2 cfg=$3
    local seed=$((SEED_BASE + wid * 1000))
    local total_violations=0
    local total_runs=0

    while [ "$(date +%s)" -lt "$END_TIME" ]; do
        total_runs=$((total_runs + 1))
        local logfile="$LOGDIR/w${wid}_s${seed}.log"

        # Batch sizes tuned for BF=3, MaxDepth=4 (~10 min per batch).
        local sim_num=5000
        case "$spec" in
            AdversarialProof) sim_num=1000 ;;
            TrieOperations)   sim_num=1000 ;;
        esac

        "$JAVA" -XX:+UseParallelGC -Xmx800m \
            -jar "$JAR" -maxSetSize 10000000 -continue \
            -simulate "num=${sim_num}" -seed "$seed" \
            -config "$cfg" "${spec}.tla" \
            > "$logfile" 2>&1

        local v
        v=$(grep -c "is violated" "$logfile" 2>/dev/null || true)
        v=${v:-0}
        if [ "$v" -gt 0 ]; then
            total_violations=$((total_violations + v))
            echo "$(date +%H:%M:%S) W${wid} ${spec}(seed=${seed}): ${v} violation(s)"
            grep -A 15 "is violated" "$logfile" >> "$COMBINED"
            echo "--- W${wid} seed=${seed} ---" >> "$COMBINED"
        fi

        seed=$((seed + 1))
    done

    echo "$(date +%H:%M:%S) W${wid} ${spec}: done. runs=${total_runs} violations=${total_violations}"
}

echo "Starting workers at $(date +%H:%M:%S)..."
echo ""

worker_loop 0 CompressedTrieModel "$LOGDIR/ctm.cfg" &
worker_loop 1 CompressedTrieModel "$LOGDIR/ctm.cfg" &
worker_loop 2 TrieOperations "$LOGDIR/trio.cfg" &
worker_loop 3 TrieOperations "$LOGDIR/trio.cfg" &
worker_loop 4 ChangeProofVerification "$LOGDIR/cpv.cfg" &
worker_loop 5 ChangeProofVerification "$LOGDIR/cpv.cfg" &
worker_loop 6 AdversarialProof "$LOGDIR/adv.cfg" &
worker_loop 7 AdversarialProof "$LOGDIR/adv.cfg" &

wait

echo ""
echo "=== Summary ==="
echo "Duration: $HOURS hours"
echo "Log directory: $LOGDIR"
if [ -f "$COMBINED" ]; then
    total=$(grep -c "is violated" "$COMBINED" 2>/dev/null || true)
    echo "Total violations: ${total:-0}"
else
    echo "Total violations: 0"
fi
