const fs = require("node:fs");
const path = require("node:path");

const BENCHMARK_DATA_PREFIX = "window.BENCHMARK_DATA = ";
const COMMENT_MARKER = "<!-- replay-10k-performance-summary -->";
const METRIC_LABELS = {
  "BenchmarkReplayLog - s/op": "Total replay",
  "BenchmarkReplayLog - s/commit": "Commit",
  "BenchmarkReplayLog - s/propose-on-db": "Propose on DB",
  "BenchmarkReplayLog - s/propose-on-proposal": "Propose on proposal",
  "BenchmarkReplayLog - s/batch": "Batch update",
  "BenchmarkReplayLog - s/get-latest": "Get latest",
};
const PREFERRED_METRICS = Object.keys(METRIC_LABELS);

function readBenchmarkData(relativeDir) {
  const repoDir = process.env.BENCHMARK_REPORTS_DIR || "benchmark-reports-repository";
  const dataPath = path.join(process.env.GITHUB_WORKSPACE, repoDir, relativeDir, "data.js");
  if (!fs.existsSync(dataPath)) {
    return null;
  }

  const content = fs.readFileSync(dataPath, "utf8");
  if (!content.startsWith(BENCHMARK_DATA_PREFIX)) {
    return null;
  }

  try {
    return JSON.parse(content.slice(BENCHMARK_DATA_PREFIX.length));
  } catch {
    return null;
  }
}

function suiteEntries(data, suiteName) {
  const entries = data?.entries?.[suiteName];
  return Array.isArray(entries) ? entries : [];
}

function findLastBySha(entries, sha) {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    if (entries[index]?.commit?.id === sha) {
      return { entry: entries[index], index };
    }
  }
  return null;
}

function findPrevious(entries, startIndex, excludedShas) {
  for (let index = startIndex - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    const sha = entry?.commit?.id;
    if (sha && !excludedShas.has(sha)) {
      return entry;
    }
  }
  return null;
}

function metricMap(entry) {
  const metrics = new Map();
  for (const bench of entry?.benches || []) {
    if (!bench || typeof bench !== "object") {
      continue;
    }
    if (typeof bench.name !== "string") {
      continue;
    }
    if (typeof bench.value !== "number" || !Number.isFinite(bench.value)) {
      continue;
    }
    metrics.set(bench.name, {
      value: bench.value,
      unit: typeof bench.unit === "string" ? bench.unit : "",
    });
  }
  return metrics;
}

function formatValue(value, unit) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "—";
  }
  if (value >= 1) {
    return `${value.toFixed(3)} ${unit}`;
  }
  if (value >= 0.01) {
    return `${value.toFixed(4)} ${unit}`;
  }
  if (value >= 0.0001) {
    return `${value.toFixed(6)} ${unit}`;
  }
  return `${value.toExponential(2)} ${unit}`;
}

function formatDelta(current, baseline) {
  if (
    typeof current !== "number" ||
    typeof baseline !== "number" ||
    !Number.isFinite(current) ||
    !Number.isFinite(baseline) ||
    baseline === 0
  ) {
    return "—";
  }

  const percent = ((current - baseline) / baseline) * 100;
  const indicator = percent <= -1 ? "🟢" : percent >= 1 ? "🔴" : "🟡";
  const sign = percent > 0 ? "+" : "";
  return `${indicator} ${sign}${percent.toFixed(2)}%`;
}

function shortSha(sha) {
  return (sha || "").slice(0, 8);
}

function commitMessage(entry) {
  return (entry?.commit?.message || "").replace(/\r?\n/g, " ");
}

function metricNames(headMetrics) {
  const names = PREFERRED_METRICS.filter((name) => headMetrics.has(name)).slice(0, 5);
  if (names.length > 0) {
    return names;
  }

  for (const name of headMetrics.keys()) {
    if (name.startsWith("BenchmarkReplayLog - ") && !name.endsWith("commits")) {
      names.push(name);
    }
    if (names.length >= 5) {
      break;
    }
  }
  return names;
}

module.exports = async function postReplayBenchmarkComment({ github, context }) {
  const prNumber = Number.parseInt(process.env.PR_NUMBER, 10);
  const baseCommitSha = process.env.BASE_COMMIT_SHA;
  const headCommitSha = process.env.HEAD_COMMIT_SHA;
  const baseBenchmarkRequested = process.env.BASE_BENCHMARK_REQUESTED === "true";
  const baseBenchmarkOutcome = process.env.BASE_BENCHMARK_OUTCOME || "skipped";
  const trendUrl = process.env.TREND_URL;
  const comparisonUrl = process.env.COMPARISON_URL;
  const repoUrl = `https://github.com/${context.repo.owner}/${context.repo.repo}`;

  const trendEntries = suiteEntries(readBenchmarkData(process.env.PR_DATA_DIR), "Replay 10K PR");
  const comparisonEntries = suiteEntries(readBenchmarkData(process.env.COMPARISON_DATA_DIR), "Replay 10K PR Compare");

  const headTrend = findLastBySha(trendEntries, headCommitSha) || (
    trendEntries.length > 0 ? { entry: trendEntries[trendEntries.length - 1], index: trendEntries.length - 1 } : null
  );
  const headComparison = findLastBySha(comparisonEntries, headCommitSha);
  const baseTrend = findLastBySha(trendEntries, baseCommitSha);
  const baseComparison = findLastBySha(comparisonEntries, baseCommitSha);
  const previousTrend = headTrend
    ? findPrevious(trendEntries, headTrend.index, new Set([headCommitSha, baseCommitSha]))
    : null;

  const headEntry = headComparison?.entry || headTrend?.entry || null;
  const baseEntry = baseComparison?.entry || baseTrend?.entry || null;
  const headMetrics = metricMap(headEntry);
  const previousMetrics = metricMap(previousTrend);
  const baseMetrics = metricMap(baseEntry);

  const rows = metricNames(headMetrics).map((name) => {
    const head = headMetrics.get(name);
    const previous = previousMetrics.get(name);
    const base = baseMetrics.get(name);
    return `| ${METRIC_LABELS[name] || name.replace(/^BenchmarkReplayLog - /, "")} | ${formatValue(head?.value, head?.unit || "")} | ${formatDelta(head?.value, previous?.value)} | ${formatDelta(head?.value, base?.value)} |`;
  });

  const commitLink = (sha) => `[${shortSha(sha)}](${repoUrl}/commit/${sha})`;
  const baseStatus = !baseBenchmarkRequested
    ? "Base data reused from history."
    : baseBenchmarkOutcome === "success"
      ? "Base benchmark executed in this run."
      : `Base benchmark did not succeed (\`${baseBenchmarkOutcome}\`).`;

  const body = [
    COMMENT_MARKER,
    "## Replay 10K Benchmark Summary",
    "",
    `**Head:** ${commitLink(headCommitSha)}${headEntry ? ` - ${commitMessage(headEntry)}` : ""}`,
    `**Base:** ${commitLink(baseCommitSha)}${baseEntry ? ` - ${commitMessage(baseEntry)}` : ""}`,
    previousTrend?.commit?.id
      ? `**Previous PR commit:** ${commitLink(previousTrend.commit.id)} - ${commitMessage(previousTrend)}`
      : "**Previous PR commit:** _Not available yet_",
    "",
    "_Lower is better for all timing metrics._",
    "",
    "| Metric | Head | vs Previous | vs Base |",
    "| --- | ---: | ---: | ---: |",
    ...(rows.length > 0 ? rows : ["| No parsed timing metrics | — | — | — |"]),
    "",
    `**Status:** ${baseStatus}`,
    "",
    `- [PR trend graph](${trendUrl})`,
    `- [Base vs head graph](${comparisonUrl})`,
  ].join("\n");

  const { data: comments } = await github.rest.issues.listComments({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: prNumber,
    per_page: 100,
  });

  const existing = comments.find(
    (comment) => comment.user?.login === "github-actions[bot]" && comment.body?.includes(COMMENT_MARKER),
  );

  if (existing) {
    await github.rest.issues.updateComment({
      owner: context.repo.owner,
      repo: context.repo.repo,
      comment_id: existing.id,
      body,
    });
    return;
  }

  await github.rest.issues.createComment({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: prNumber,
    body,
  });
};
