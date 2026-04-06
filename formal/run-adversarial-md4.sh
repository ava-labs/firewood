#!/bin/bash
# Run AdversarialProof at BF=3, MaxDepth=4 with single-key tampering.
# Full enumeration (OnlyCorrectDiffAccepted) is infeasible at this depth.
#
# Usage: ./run-adversarial-md4.sh [hours] [num_workers]

set -e

HOURS="${1:-2}"
WORKERS="${2:-2}"
DURATION=$((HOURS * 3600))
JAVA=/opt/homebrew/opt/openjdk/bin/java
JAR="$(dirname "$0")/tla2tools.jar"
LOGDIR="/tmp/adv_md4_$(date +%s)"
COMBINED="$LOGDIR/all_violations.log"

mkdir -p "$LOGDIR"
cd "$(dirname "$0")"

echo "AdversarialProof BF=3 MaxDepth=4 (single-key tampering)"
echo "Running $WORKERS workers for $HOURS hours"
echo "Logs in $LOGDIR"
echo ""

END_TIME=$(($(date +%s) + DURATION))
SEED_BASE=500

cat > "$LOGDIR/adv_md4.cfg" << 'EOF'
CONSTANTS
    BF = 3
    MaxDepth = 4
    NumValues = 2
    None = None
INIT SimInit
NEXT Next
INVARIANT
    HonestProofAccepted
    AdversarialProofRejected
EOF

worker_loop() {
    local wid=$1
    local seed=$((SEED_BASE + wid * 1000))
    local total_violations=0
    local total_runs=0

    while [ "$(date +%s)" -lt "$END_TIME" ]; do
        total_runs=$((total_runs + 1))
        local logfile="$LOGDIR/w${wid}_s${seed}.log"

        "$JAVA" -XX:+UseParallelGC -Xmx800m \
            -jar "$JAR" -maxSetSize 10000000 -continue \
            -simulate "num=1000" -seed "$seed" \
            -config "$LOGDIR/adv_md4.cfg" AdversarialProof.tla \
            > "$logfile" 2>&1

        local v
        v=$(grep -c "is violated" "$logfile" 2>/dev/null || true)
        v=${v:-0}
        if [ "$v" -gt 0 ]; then
            total_violations=$((total_violations + v))
            echo "$(date +%H:%M:%S) W${wid} seed=${seed}: ${v} violation(s)"
            grep -A 15 "is violated" "$logfile" >> "$COMBINED"
            echo "--- W${wid} seed=${seed} ---" >> "$COMBINED"
        fi

        seed=$((seed + 1))
    done

    echo "$(date +%H:%M:%S) W${wid}: done. runs=${total_runs} violations=${total_violations}"
}

echo "Starting workers at $(date +%H:%M:%S)..."
echo ""

for ((i=0; i<WORKERS; i++)); do
    worker_loop $i &
done

wait

echo ""
echo "=== Summary ==="
echo "Duration: $HOURS hours"
echo "Workers: $WORKERS"
echo "Log directory: $LOGDIR"
if [ -f "$COMBINED" ]; then
    total=$(grep -c "is violated" "$COMBINED" 2>/dev/null || true)
    echo "Total violations: ${total:-0}"
else
    echo "Total violations: 0"
fi
