#!/bin/bash
JAVA=/opt/homebrew/opt/openjdk/bin/java
JAR="$(dirname "$0")/tla2tools.jar"
cd "$(dirname "$0")"
LOGDIR="/tmp/adversarial_bf3_$(date +%s)"
mkdir -p "$LOGDIR"
echo "BF=3 adversarial logs in $LOGDIR"

SEED=100
END_TIME=$(($(date +%s) + 7200))

for worker in 0 1; do
    (
        while [ "$(date +%s)" -lt "$END_TIME" ]; do
            log="$LOGDIR/w${worker}_s${SEED}.log"
            "$JAVA" -XX:+UseParallelGC -Xmx800m \
                -jar "$JAR" -maxSetSize 10000000 -continue \
                -simulate "num=1000000000" -seed "$SEED" \
                -config AdversarialProof-bf3.cfg AdversarialProof.tla \
                > "$log" 2>&1
            v=$(grep -c "is violated" "$log" 2>/dev/null || echo 0)
            t=$(grep "traces generated" "$log" | tail -1 | grep -o "[0-9]* traces" | grep -o "[0-9]*" || echo 0)
            if [ "$v" -gt 0 ]; then
                echo "$(date +%H:%M:%S) BF3 worker $worker seed $SEED: $v violation(s) after $t traces"
                grep -A 20 "is violated" "$log" >> "$LOGDIR/all_violations.log"
            fi
            SEED=$((SEED + 2))
        done
    ) &
    echo "BF=3 worker $worker started"
    SEED=$((SEED + 1))
done
wait
echo "BF=3 adversarial run complete"
echo "Violations: $(grep -c 'is violated' "$LOGDIR/all_violations.log" 2>/dev/null || echo 0)"
