#!/bin/bash
set -e
JAVA=/opt/homebrew/opt/openjdk/bin/java
JAR="$(dirname "$0")/tla2tools.jar"
SPEC="${1:-ChangeProofVerification}"
NUM="${2:-1000000}"

cd "$(dirname "$0")"
exec "$JAVA" -XX:+UseParallelGC -jar "$JAR" \
    -maxSetSize 10000000 \
    -simulate "num=${NUM}" \
    -config "${SPEC}-sim.cfg" "${SPEC}.tla"
