#!/bin/bash
# Run multiple parallel AdversarialProof simulations for a given duration.
# When an instance hits a violation and stops, restart it with a new seed.
# All violations are logged to a combined output file.
#
# Usage: ./run-adversarial-long.sh [hours] [num_workers]
#   hours       — how long to run (default: 2)
#   num_workers — parallel instances (default: 8)

set -e

HOURS="${1:-2}"
WORKERS="${2:-8}"
DURATION=$((HOURS * 3600))
JAVA=/opt/homebrew/opt/openjdk/bin/java
JAR="$(dirname "$0")/tla2tools.jar"
SPEC="AdversarialProof"
CFG="AdversarialProof-sim.cfg"
LOGDIR="/tmp/adversarial_long_$(date +%s)"
COMBINED="$LOGDIR/all_violations.log"

mkdir -p "$LOGDIR"
cd "$(dirname "$0")"

echo "Running $WORKERS workers for $HOURS hours"
echo "Logs in $LOGDIR"
echo "Violations logged to $COMBINED"
echo ""

SEED_COUNTER=1
END_TIME=$(($(date +%s) + DURATION))

# Start a worker. Args: worker_id seed
start_worker() {
    local wid=$1
    local seed=$2
    local logfile="$LOGDIR/worker${wid}_seed${seed}.log"

    "$JAVA" -XX:+UseParallelGC -Xmx800m \
        -jar "$JAR" -maxSetSize 10000000 -continue \
        -simulate "num=1000000000" -seed "$seed" \
        -config "$CFG" "$SPEC.tla" \
        > "$logfile" 2>&1 &

    echo $!
}

# Track worker PIDs
declare -a PIDS
declare -a WORKER_SEEDS

# Launch initial workers
for ((i=0; i<WORKERS; i++)); do
    PIDS[$i]=$(start_worker $i $SEED_COUNTER)
    WORKER_SEEDS[$i]=$SEED_COUNTER
    echo "Worker $i started with seed $SEED_COUNTER (PID ${PIDS[$i]})"
    SEED_COUNTER=$((SEED_COUNTER + 1))
done

TOTAL_VIOLATIONS=0
TOTAL_TRACES=0

# Monitor loop
while [ "$(date +%s)" -lt "$END_TIME" ]; do
    sleep 10

    for ((i=0; i<WORKERS; i++)); do
        pid=${PIDS[$i]}
        seed=${WORKER_SEEDS[$i]}

        # Check if process is still running
        if ! kill -0 "$pid" 2>/dev/null; then
            # Process ended — check for violations
            logfile="$LOGDIR/worker${i}_seed${seed}.log"
            violations=$(grep -c "is violated" "$logfile" 2>/dev/null || echo 0)
            traces=$(grep "traces generated" "$logfile" 2>/dev/null | tail -1 | grep -o "[0-9]* traces" | grep -o "[0-9]*" || echo 0)

            if [ "$violations" -gt 0 ]; then
                TOTAL_VIOLATIONS=$((TOTAL_VIOLATIONS + violations))
                echo "$(date +%H:%M:%S) Worker $i (seed $seed): $violations violation(s) after $traces traces — restarting"
                # Append violations to combined log
                grep -A 20 "is violated" "$logfile" >> "$COMBINED"
                echo "---" >> "$COMBINED"
            fi

            TOTAL_TRACES=$((TOTAL_TRACES + traces))

            # Restart with new seed if time remains
            if [ "$(date +%s)" -lt "$END_TIME" ]; then
                PIDS[$i]=$(start_worker $i $SEED_COUNTER)
                WORKER_SEEDS[$i]=$SEED_COUNTER
                SEED_COUNTER=$((SEED_COUNTER + 1))
            fi
        fi
    done
done

echo ""
echo "Time's up. Stopping workers..."

# Kill remaining workers and collect final stats
for ((i=0; i<WORKERS; i++)); do
    pid=${PIDS[$i]}
    seed=${WORKER_SEEDS[$i]}
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true

    logfile="$LOGDIR/worker${i}_seed${seed}.log"
    traces=$(grep "traces generated" "$logfile" 2>/dev/null | tail -1 | grep -o "[0-9]* traces" | grep -o "[0-9]*" || echo 0)
    TOTAL_TRACES=$((TOTAL_TRACES + traces))
done

# Count unique violation patterns
UNIQUE=0
if [ -f "$COMBINED" ]; then
    UNIQUE=$(grep -c "is violated" "$COMBINED" 2>/dev/null || echo 0)
fi

echo ""
echo "=== Summary ==="
echo "Duration: $HOURS hours"
echo "Workers: $WORKERS"
echo "Seeds used: $((SEED_COUNTER - 1))"
echo "Total traces: $TOTAL_TRACES"
echo "Total violations: $((TOTAL_VIOLATIONS + UNIQUE - UNIQUE))"
echo "Violations log: $COMBINED"
