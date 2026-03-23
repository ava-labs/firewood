#!/usr/bin/env python3

import argparse
import json
import math
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

PREFIX = "window.BENCHMARK_DATA = "


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    for name in ("repository", "branch", "token", "theme-css-url", "commit-message"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--workdir", default="benchmark-reports-repository")
    parser.add_argument("--data-dir", action="append", default=[])
    parser.add_argument("--commit-subject", action="append", nargs=2, metavar=("SHA", "SUBJECT"), default=[])
    return parser.parse_args()


def git(*args: str, cwd: Path | None = None) -> None:
    subprocess.run(["git", *args], check=True, cwd=cwd)


def load_benchmark_data(path: Path) -> dict | None:
    if not path.is_file():
        return None

    text = path.read_text(encoding="utf-8")
    if not text.startswith(PREFIX):
        return None

    try:
        return json.loads(text[len(PREFIX) :])
    except json.JSONDecodeError:
        return None


def convert_unit(unit: object) -> str | None:
    if unit == "ns":
        return "s"
    if isinstance(unit, str) and unit.startswith("ns/"):
        return "s/" + unit[3:]
    return "s/op" if unit == "ns/op" else None


def normalize_benchmark_data(path: Path, commit_subjects: dict[str, str]) -> None:
    data = load_benchmark_data(path)
    if data is None:
        return

    entries = data.get("entries")
    if not isinstance(entries, dict):
        return

    changed = False
    for suites in entries.values():
        if not isinstance(suites, list):
            continue
        for suite in suites:
            if not isinstance(suite, dict):
                continue

            commit = suite.get("commit")
            if isinstance(commit, dict):
                subject = commit_subjects.get(commit.get("id"))
                if subject and commit.get("message") != subject:
                    commit["message"] = subject
                    changed = True

            benches = suite.get("benches")
            if not isinstance(benches, list):
                continue

            filtered = []
            for bench in benches:
                if not isinstance(bench, dict):
                    filtered.append(bench)
                    continue
                if bench.get("name") == "BenchmarkReplayLog":
                    changed = True
                    continue

                converted_unit = convert_unit(bench.get("unit"))
                if converted_unit:
                    if isinstance(value := bench.get("value"), (int, float)) and math.isfinite(value):
                        bench["value"] = value / 1_000_000_000
                    bench["unit"] = converted_unit

                    if isinstance(name := bench.get("name"), str):
                        bench["name"] = (
                            name.replace(" - ns/op", " - s/op").replace(" - ns/", " - s/").replace(" - ns", " - s")
                        )
                    changed = True

                filtered.append(bench)

            if len(filtered) != len(benches):
                suite["benches"] = filtered

    if changed:
        path.write_text(PREFIX + json.dumps(data, indent=2), encoding="utf-8")


def apply_theme(report_dir: Path, theme_css: str) -> None:
    index_path = report_dir / "index.html"
    if not index_path.is_file():
        print(f"::warning::Skipping {report_dir} because index.html is missing")
        return

    (report_dir / "theme.css").write_text(theme_css, encoding="utf-8")

    html = index_path.read_text(encoding="utf-8")
    if 'href="theme.css"' in html:
        return

    if "</head>" not in html:
        print(f"::warning::Skipping {index_path} because </head> is missing")
        return

    index_path.write_text(
        html.replace("</head>", '  <link rel="stylesheet" href="theme.css">\n</head>', 1),
        encoding="utf-8",
    )


def main() -> int:
    args = parse_args()
    workdir = Path(args.workdir)
    commit_subjects = dict(args.commit_subject)

    shutil.rmtree(workdir, ignore_errors=True)
    clone_url = f"https://x-access-token:{args.token}@github.com/{args.repository}.git"
    git("clone", "--depth", "1", "--branch", args.branch, clone_url, str(workdir))

    with urllib.request.urlopen(args.theme_css_url) as response:
        theme_css = response.read().decode("utf-8")

    for data_dir in args.data_dir:
        report_dir = workdir / data_dir
        normalize_benchmark_data(report_dir / "data.js", commit_subjects)
        apply_theme(report_dir, theme_css)

    git("config", "user.name", "github-actions[bot]", cwd=workdir)
    git("config", "user.email", "github-actions[bot]@users.noreply.github.com", cwd=workdir)
    git("add", "-A", ".", cwd=workdir)

    diff = subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=workdir, check=False)
    if diff.returncode == 0:
        print("No benchmark report changes to commit.")
        return 0
    if diff.returncode != 1:
        return diff.returncode

    git("commit", "-m", args.commit_message, cwd=workdir)
    git("push", "origin", args.branch, cwd=workdir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
