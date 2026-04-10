window.BENCHMARK_DATA = {
  "lastUpdate": 1775807002678,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ef50158655c20bfb81c1fc31279973e36c639846",
          "message": "chore(deps): bump bytes from 1.11.0 to 1.11.1 (#1654)\n\nBumps [bytes](https://github.com/tokio-rs/bytes) from 1.11.0 to 1.11.1.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/tokio-rs/bytes/releases\">bytes's\nreleases</a>.</em></p>\n<blockquote>\n<h2>Bytes v1.11.1</h2>\n<h1>1.11.1 (February 3rd, 2026)</h1>\n<ul>\n<li>Fix integer overflow in <code>BytesMut::reserve</code></li>\n</ul>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/tokio-rs/bytes/blob/master/CHANGELOG.md\">bytes's\nchangelog</a>.</em></p>\n<blockquote>\n<h1>1.11.1 (February 3rd, 2026)</h1>\n<ul>\n<li>Fix integer overflow in <code>BytesMut::reserve</code></li>\n</ul>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/tokio-rs/bytes/commit/417dccdeff249e0c011327de7d92e0d6fbe7cc43\"><code>417dccd</code></a>\nRelease bytes v1.11.1 (<a\nhref=\"https://redirect.github.com/tokio-rs/bytes/issues/820\">#820</a>)</li>\n<li><a\nhref=\"https://github.com/tokio-rs/bytes/commit/d0293b0e35838123c51ca5dfdf468ecafee4398f\"><code>d0293b0</code></a>\nMerge commit from fork</li>\n<li>See full diff in <a\nhref=\"https://github.com/tokio-rs/bytes/compare/v1.11.0...v1.11.1\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=bytes&package-manager=cargo&previous-version=1.11.0&new-version=1.11.1)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot merge` will merge this PR after your CI passes on it\n- `@dependabot squash and merge` will squash and merge this PR after\nyour CI passes on it\n- `@dependabot cancel merge` will cancel a previously requested merge\nand block automerging\n- `@dependabot reopen` will reopen this PR if it is closed\n- `@dependabot close` will close this PR and stop Dependabot recreating\nit. You can achieve the same result by closing it manually\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-02-03T21:13:05Z",
          "url": "https://github.com/ava-labs/firewood/commit/ef50158655c20bfb81c1fc31279973e36c639846"
        },
        "date": 1770153928498,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 141.10462769442637,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7086.939786025885,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 118.63327372299207,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6864.530240005926,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 99.52880453102385,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "21dc752a25dc41af7eba78bfa335e1a90637289f",
          "message": "fix(ffi): force correct alignment on certain c structs (#1652)\n\n## Why this should be merged\n\nWhen enabling race detection in Go, it alerted us that Go was producing\nstructs that were not properly aligned. This was due to the union part\nbeing reduced down to a byte array which always has an alignment of 1.\nThe enum part had an automatic alignment of 4 bytes, resulting in a\n4-byte padding between the tag and the union as well as a 4-byte\nalignment when the struct was created in Go. The 4-byte alignment is\nincorrect on 64-bit platforms and should be pointer-width alignment for\nthe tagged enums that contain pointers.\n\n## How this works\n\nThis resolves the issue by explicitly setting the alignment of the tag\npart of the tagged union structs to be pointer-width aligned. This is\nflexible enough to handle both 32-bit and 64-bit architectures as well\nas any future 128-bit architectures. I included all enums with pointers\nin their bodies and not just the ones created in Go for consistency and\nto reduce ambiguity.\n\n## How this was tested\n\nRace detection was enabled in the tests, which no longer fails.\n\nCloses: #1651",
          "timestamp": "2026-02-03T21:33:35Z",
          "url": "https://github.com/ava-labs/firewood/commit/21dc752a25dc41af7eba78bfa335e1a90637289f"
        },
        "date": 1770191058584,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 143.20966530170773,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6982.768920611919,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 113.61592016820593,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6774.77296312794,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 90.95787458298983,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "61739ef8f3cd1ee802d53a503baa81faf0dcc036",
          "message": "chore: update to go 1.24.12 (#1658)\n\n## Why this should be merged\n\nAvalancheGo recently bumped their version of Golang. Bumping our version\nof Golang also facilitates running reexecution tests with deferred\npersistence.\n\n## How this works\n\nFollowed the instructions in `ffi/go.mod`.\n\n## How this was tested\n\nCI",
          "timestamp": "2026-02-04T19:41:48Z",
          "url": "https://github.com/ava-labs/firewood/commit/61739ef8f3cd1ee802d53a503baa81faf0dcc036"
        },
        "date": 1770277366977,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 150.18572198432344,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6658.422563660088,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.19892430072916,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6466.382788914033,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 81.33213717039682,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8a19798adf8990bb7b2de119a178867c38f9d73e",
          "message": "chore(deps): bump time from 0.3.45 to 0.3.47 (#1660)\n\nBumps [time](https://github.com/time-rs/time) from 0.3.45 to 0.3.47.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/releases\">time's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v0.3.47</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n<h2>v0.3.46</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">time's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>0.3.47 [2026-02-05]</h2>\n<h3>Security</h3>\n<ul>\n<li>\n<p>The possibility of a stack exhaustion denial of service attack when\nparsing RFC 2822 has been\neliminated. Previously, it was possible to craft input that would cause\nunbounded recursion. Now,\nthe depth of the recursion is tracked, causing an error to be returned\nif it exceeds a reasonable\nlimit.</p>\n<p>This attack vector requires parsing user-provided input, with any\ntype, using the RFC 2822 format.</p>\n</li>\n</ul>\n<h3>Compatibility</h3>\n<ul>\n<li>Attempting to format a value with a well-known format (i.e. RFC\n3339, RFC 2822, or ISO 8601) will\nerror at compile time if the type being formatted does not provide\nsufficient information. This\nwould previously fail at runtime. Similarly, attempting to format a\nvalue with ISO 8601 that is\nonly configured for parsing (i.e. <code>Iso8601::PARSING</code>) will\nerror at compile time.</li>\n</ul>\n<h3>Added</h3>\n<ul>\n<li>Builder methods for format description modifiers, eliminating the\nneed for verbose initialization\nwhen done manually.</li>\n<li><code>date!(2026-W01-2)</code> is now supported. Previously, a space\nwas required between <code>W</code> and <code>01</code>.</li>\n<li><code>[end]</code> now has a <code>trailing_input</code> modifier\nwhich can either be <code>prohibit</code> (the default) or\n<code>discard</code>. When it is <code>discard</code>, all remaining\ninput is ignored. Note that if there are components\nafter <code>[end]</code>, they will still attempt to be parsed, likely\nresulting in an error.</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>More performance gains when parsing.</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>If manually formatting a value, the number of bytes written was one\nshort for some components.\nThis has been fixed such that the number of bytes written is always\ncorrect.</li>\n<li>The possibility of integer overflow when parsing an owned format\ndescription has been effectively\neliminated. This would previously wrap when overflow checks were\ndisabled. Instead of storing the\ndepth as <code>u8</code>, it is stored as <code>u32</code>. This would\nrequire multiple gigabytes of nested input to\noverflow, at which point we've got other problems and trivial\nmitigations are available by\ndownstream users.</li>\n</ul>\n<h2>0.3.46 [2026-01-23]</h2>\n<h3>Added</h3>\n<ul>\n<li>All possible panics are now documented for the relevant\nmethods.</li>\n<li>The need to use <code>#[serde(default)]</code> when using custom\n<code>serde</code> formats is documented. This applies\nonly when deserializing an <code>Option&lt;T&gt;</code>.</li>\n<li><code>Duration::nanoseconds_i128</code> has been made public,\nmirroring\n<code>std::time::Duration::from_nanos_u128</code>.</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/d5144cd2874862d46466c900910cd8577d066019\"><code>d5144cd</code></a>\nv0.3.47 release</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/f6206b050fd54817d8872834b4d61f605570e89b\"><code>f6206b0</code></a>\nGuard against integer overflow in release mode</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/1c63dc7985b8fa26bd8c689423cc56b7a03841ee\"><code>1c63dc7</code></a>\nAvoid denial of service when parsing Rfc2822</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/5940df6e72efb63d246ca1ca59a0f836ad32ad8a\"><code>5940df6</code></a>\nAdd builder methods to avoid verbose construction</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/00881a4da1bc5a6cb6313052e5017dbd7daa40f0\"><code>00881a4</code></a>\nManually format macros everywhere</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/bb723b6d826e46c174d75cd08987061984b0ceb7\"><code>bb723b6</code></a>\nAdd <code>trailing_input</code> modifier to <code>end</code></li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/31c4f8e0b56e6ae24fe0d6ef0e492b6741dda783\"><code>31c4f8e</code></a>\nPermit <code>W12</code> in <code>date!</code> macro</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/490a17bf306576850f33a86d3ca95d96db7b1dcd\"><code>490a17b</code></a>\nMark error paths in well-known formats as cold</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6cb1896a600be1538ecfab8f233fe9cfe9fa8951\"><code>6cb1896</code></a>\nOptimize <code>Rfc2822</code> parsing</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6d264d59c25e3da0453c3defebf4640b0086a006\"><code>6d264d5</code></a>\nRemove erroneous <code>#[inline(never)]</code> attributes</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/time-rs/time/compare/v0.3.45...v0.3.47\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=time&package-manager=cargo&previous-version=0.3.45&new-version=0.3.47)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-02-05T19:18:52Z",
          "url": "https://github.com/ava-labs/firewood/commit/8a19798adf8990bb7b2de119a178867c38f9d73e"
        },
        "date": 1770384850370,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 146.2020247565875,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6839.8505538135005,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.7023334568076,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6642.167470706135,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 84.20875050371791,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8a19798adf8990bb7b2de119a178867c38f9d73e",
          "message": "chore(deps): bump time from 0.3.45 to 0.3.47 (#1660)\n\nBumps [time](https://github.com/time-rs/time) from 0.3.45 to 0.3.47.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/releases\">time's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v0.3.47</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n<h2>v0.3.46</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">time's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>0.3.47 [2026-02-05]</h2>\n<h3>Security</h3>\n<ul>\n<li>\n<p>The possibility of a stack exhaustion denial of service attack when\nparsing RFC 2822 has been\neliminated. Previously, it was possible to craft input that would cause\nunbounded recursion. Now,\nthe depth of the recursion is tracked, causing an error to be returned\nif it exceeds a reasonable\nlimit.</p>\n<p>This attack vector requires parsing user-provided input, with any\ntype, using the RFC 2822 format.</p>\n</li>\n</ul>\n<h3>Compatibility</h3>\n<ul>\n<li>Attempting to format a value with a well-known format (i.e. RFC\n3339, RFC 2822, or ISO 8601) will\nerror at compile time if the type being formatted does not provide\nsufficient information. This\nwould previously fail at runtime. Similarly, attempting to format a\nvalue with ISO 8601 that is\nonly configured for parsing (i.e. <code>Iso8601::PARSING</code>) will\nerror at compile time.</li>\n</ul>\n<h3>Added</h3>\n<ul>\n<li>Builder methods for format description modifiers, eliminating the\nneed for verbose initialization\nwhen done manually.</li>\n<li><code>date!(2026-W01-2)</code> is now supported. Previously, a space\nwas required between <code>W</code> and <code>01</code>.</li>\n<li><code>[end]</code> now has a <code>trailing_input</code> modifier\nwhich can either be <code>prohibit</code> (the default) or\n<code>discard</code>. When it is <code>discard</code>, all remaining\ninput is ignored. Note that if there are components\nafter <code>[end]</code>, they will still attempt to be parsed, likely\nresulting in an error.</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>More performance gains when parsing.</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>If manually formatting a value, the number of bytes written was one\nshort for some components.\nThis has been fixed such that the number of bytes written is always\ncorrect.</li>\n<li>The possibility of integer overflow when parsing an owned format\ndescription has been effectively\neliminated. This would previously wrap when overflow checks were\ndisabled. Instead of storing the\ndepth as <code>u8</code>, it is stored as <code>u32</code>. This would\nrequire multiple gigabytes of nested input to\noverflow, at which point we've got other problems and trivial\nmitigations are available by\ndownstream users.</li>\n</ul>\n<h2>0.3.46 [2026-01-23]</h2>\n<h3>Added</h3>\n<ul>\n<li>All possible panics are now documented for the relevant\nmethods.</li>\n<li>The need to use <code>#[serde(default)]</code> when using custom\n<code>serde</code> formats is documented. This applies\nonly when deserializing an <code>Option&lt;T&gt;</code>.</li>\n<li><code>Duration::nanoseconds_i128</code> has been made public,\nmirroring\n<code>std::time::Duration::from_nanos_u128</code>.</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/d5144cd2874862d46466c900910cd8577d066019\"><code>d5144cd</code></a>\nv0.3.47 release</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/f6206b050fd54817d8872834b4d61f605570e89b\"><code>f6206b0</code></a>\nGuard against integer overflow in release mode</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/1c63dc7985b8fa26bd8c689423cc56b7a03841ee\"><code>1c63dc7</code></a>\nAvoid denial of service when parsing Rfc2822</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/5940df6e72efb63d246ca1ca59a0f836ad32ad8a\"><code>5940df6</code></a>\nAdd builder methods to avoid verbose construction</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/00881a4da1bc5a6cb6313052e5017dbd7daa40f0\"><code>00881a4</code></a>\nManually format macros everywhere</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/bb723b6d826e46c174d75cd08987061984b0ceb7\"><code>bb723b6</code></a>\nAdd <code>trailing_input</code> modifier to <code>end</code></li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/31c4f8e0b56e6ae24fe0d6ef0e492b6741dda783\"><code>31c4f8e</code></a>\nPermit <code>W12</code> in <code>date!</code> macro</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/490a17bf306576850f33a86d3ca95d96db7b1dcd\"><code>490a17b</code></a>\nMark error paths in well-known formats as cold</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6cb1896a600be1538ecfab8f233fe9cfe9fa8951\"><code>6cb1896</code></a>\nOptimize <code>Rfc2822</code> parsing</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6d264d59c25e3da0453c3defebf4640b0086a006\"><code>6d264d5</code></a>\nRemove erroneous <code>#[inline(never)]</code> attributes</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/time-rs/time/compare/v0.3.45...v0.3.47\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=time&package-manager=cargo&previous-version=0.3.45&new-version=0.3.47)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-02-05T19:18:52Z",
          "url": "https://github.com/ava-labs/firewood/commit/8a19798adf8990bb7b2de119a178867c38f9d73e"
        },
        "date": 1770537484566,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 134.87986138192977,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7414.005246998074,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 127.66903808739987,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7169.745766762266,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 111.4784556102861,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8a19798adf8990bb7b2de119a178867c38f9d73e",
          "message": "chore(deps): bump time from 0.3.45 to 0.3.47 (#1660)\n\nBumps [time](https://github.com/time-rs/time) from 0.3.45 to 0.3.47.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/releases\">time's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v0.3.47</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n<h2>v0.3.46</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">time's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>0.3.47 [2026-02-05]</h2>\n<h3>Security</h3>\n<ul>\n<li>\n<p>The possibility of a stack exhaustion denial of service attack when\nparsing RFC 2822 has been\neliminated. Previously, it was possible to craft input that would cause\nunbounded recursion. Now,\nthe depth of the recursion is tracked, causing an error to be returned\nif it exceeds a reasonable\nlimit.</p>\n<p>This attack vector requires parsing user-provided input, with any\ntype, using the RFC 2822 format.</p>\n</li>\n</ul>\n<h3>Compatibility</h3>\n<ul>\n<li>Attempting to format a value with a well-known format (i.e. RFC\n3339, RFC 2822, or ISO 8601) will\nerror at compile time if the type being formatted does not provide\nsufficient information. This\nwould previously fail at runtime. Similarly, attempting to format a\nvalue with ISO 8601 that is\nonly configured for parsing (i.e. <code>Iso8601::PARSING</code>) will\nerror at compile time.</li>\n</ul>\n<h3>Added</h3>\n<ul>\n<li>Builder methods for format description modifiers, eliminating the\nneed for verbose initialization\nwhen done manually.</li>\n<li><code>date!(2026-W01-2)</code> is now supported. Previously, a space\nwas required between <code>W</code> and <code>01</code>.</li>\n<li><code>[end]</code> now has a <code>trailing_input</code> modifier\nwhich can either be <code>prohibit</code> (the default) or\n<code>discard</code>. When it is <code>discard</code>, all remaining\ninput is ignored. Note that if there are components\nafter <code>[end]</code>, they will still attempt to be parsed, likely\nresulting in an error.</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>More performance gains when parsing.</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>If manually formatting a value, the number of bytes written was one\nshort for some components.\nThis has been fixed such that the number of bytes written is always\ncorrect.</li>\n<li>The possibility of integer overflow when parsing an owned format\ndescription has been effectively\neliminated. This would previously wrap when overflow checks were\ndisabled. Instead of storing the\ndepth as <code>u8</code>, it is stored as <code>u32</code>. This would\nrequire multiple gigabytes of nested input to\noverflow, at which point we've got other problems and trivial\nmitigations are available by\ndownstream users.</li>\n</ul>\n<h2>0.3.46 [2026-01-23]</h2>\n<h3>Added</h3>\n<ul>\n<li>All possible panics are now documented for the relevant\nmethods.</li>\n<li>The need to use <code>#[serde(default)]</code> when using custom\n<code>serde</code> formats is documented. This applies\nonly when deserializing an <code>Option&lt;T&gt;</code>.</li>\n<li><code>Duration::nanoseconds_i128</code> has been made public,\nmirroring\n<code>std::time::Duration::from_nanos_u128</code>.</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/d5144cd2874862d46466c900910cd8577d066019\"><code>d5144cd</code></a>\nv0.3.47 release</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/f6206b050fd54817d8872834b4d61f605570e89b\"><code>f6206b0</code></a>\nGuard against integer overflow in release mode</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/1c63dc7985b8fa26bd8c689423cc56b7a03841ee\"><code>1c63dc7</code></a>\nAvoid denial of service when parsing Rfc2822</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/5940df6e72efb63d246ca1ca59a0f836ad32ad8a\"><code>5940df6</code></a>\nAdd builder methods to avoid verbose construction</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/00881a4da1bc5a6cb6313052e5017dbd7daa40f0\"><code>00881a4</code></a>\nManually format macros everywhere</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/bb723b6d826e46c174d75cd08987061984b0ceb7\"><code>bb723b6</code></a>\nAdd <code>trailing_input</code> modifier to <code>end</code></li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/31c4f8e0b56e6ae24fe0d6ef0e492b6741dda783\"><code>31c4f8e</code></a>\nPermit <code>W12</code> in <code>date!</code> macro</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/490a17bf306576850f33a86d3ca95d96db7b1dcd\"><code>490a17b</code></a>\nMark error paths in well-known formats as cold</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6cb1896a600be1538ecfab8f233fe9cfe9fa8951\"><code>6cb1896</code></a>\nOptimize <code>Rfc2822</code> parsing</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6d264d59c25e3da0453c3defebf4640b0086a006\"><code>6d264d5</code></a>\nRemove erroneous <code>#[inline(never)]</code> attributes</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/time-rs/time/compare/v0.3.45...v0.3.47\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=time&package-manager=cargo&previous-version=0.3.45&new-version=0.3.47)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-02-05T19:18:52Z",
          "url": "https://github.com/ava-labs/firewood/commit/8a19798adf8990bb7b2de119a178867c38f9d73e"
        },
        "date": 1770623609781,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 142.4292524844643,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7021.029616855403,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 116.19846304903882,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6806.705982371719,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 94.57573392001278,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8a19798adf8990bb7b2de119a178867c38f9d73e",
          "message": "chore(deps): bump time from 0.3.45 to 0.3.47 (#1660)",
          "timestamp": "2026-02-05T19:18:52Z",
          "url": "https://github.com/ava-labs/firewood/commit/8a19798adf8990bb7b2de119a178867c38f9d73e"
        },
        "date": 1770613200000,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[50000001,60000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 162.52731726658928,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[50000001,60000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6152.811827686333,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[50000001,60000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 71.67161754565576,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[50000001,60000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5907.025244505082,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[50000001,60000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 171.98626125910488,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8a19798adf8990bb7b2de119a178867c38f9d73e",
          "message": "chore(deps): bump time from 0.3.45 to 0.3.47 (#1660)\n\nBumps [time](https://github.com/time-rs/time) from 0.3.45 to 0.3.47.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/releases\">time's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v0.3.47</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n<h2>v0.3.46</h2>\n<p>See the <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">changelog</a>\nfor details.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/time-rs/time/blob/main/CHANGELOG.md\">time's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>0.3.47 [2026-02-05]</h2>\n<h3>Security</h3>\n<ul>\n<li>\n<p>The possibility of a stack exhaustion denial of service attack when\nparsing RFC 2822 has been\neliminated. Previously, it was possible to craft input that would cause\nunbounded recursion. Now,\nthe depth of the recursion is tracked, causing an error to be returned\nif it exceeds a reasonable\nlimit.</p>\n<p>This attack vector requires parsing user-provided input, with any\ntype, using the RFC 2822 format.</p>\n</li>\n</ul>\n<h3>Compatibility</h3>\n<ul>\n<li>Attempting to format a value with a well-known format (i.e. RFC\n3339, RFC 2822, or ISO 8601) will\nerror at compile time if the type being formatted does not provide\nsufficient information. This\nwould previously fail at runtime. Similarly, attempting to format a\nvalue with ISO 8601 that is\nonly configured for parsing (i.e. <code>Iso8601::PARSING</code>) will\nerror at compile time.</li>\n</ul>\n<h3>Added</h3>\n<ul>\n<li>Builder methods for format description modifiers, eliminating the\nneed for verbose initialization\nwhen done manually.</li>\n<li><code>date!(2026-W01-2)</code> is now supported. Previously, a space\nwas required between <code>W</code> and <code>01</code>.</li>\n<li><code>[end]</code> now has a <code>trailing_input</code> modifier\nwhich can either be <code>prohibit</code> (the default) or\n<code>discard</code>. When it is <code>discard</code>, all remaining\ninput is ignored. Note that if there are components\nafter <code>[end]</code>, they will still attempt to be parsed, likely\nresulting in an error.</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>More performance gains when parsing.</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>If manually formatting a value, the number of bytes written was one\nshort for some components.\nThis has been fixed such that the number of bytes written is always\ncorrect.</li>\n<li>The possibility of integer overflow when parsing an owned format\ndescription has been effectively\neliminated. This would previously wrap when overflow checks were\ndisabled. Instead of storing the\ndepth as <code>u8</code>, it is stored as <code>u32</code>. This would\nrequire multiple gigabytes of nested input to\noverflow, at which point we've got other problems and trivial\nmitigations are available by\ndownstream users.</li>\n</ul>\n<h2>0.3.46 [2026-01-23]</h2>\n<h3>Added</h3>\n<ul>\n<li>All possible panics are now documented for the relevant\nmethods.</li>\n<li>The need to use <code>#[serde(default)]</code> when using custom\n<code>serde</code> formats is documented. This applies\nonly when deserializing an <code>Option&lt;T&gt;</code>.</li>\n<li><code>Duration::nanoseconds_i128</code> has been made public,\nmirroring\n<code>std::time::Duration::from_nanos_u128</code>.</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/d5144cd2874862d46466c900910cd8577d066019\"><code>d5144cd</code></a>\nv0.3.47 release</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/f6206b050fd54817d8872834b4d61f605570e89b\"><code>f6206b0</code></a>\nGuard against integer overflow in release mode</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/1c63dc7985b8fa26bd8c689423cc56b7a03841ee\"><code>1c63dc7</code></a>\nAvoid denial of service when parsing Rfc2822</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/5940df6e72efb63d246ca1ca59a0f836ad32ad8a\"><code>5940df6</code></a>\nAdd builder methods to avoid verbose construction</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/00881a4da1bc5a6cb6313052e5017dbd7daa40f0\"><code>00881a4</code></a>\nManually format macros everywhere</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/bb723b6d826e46c174d75cd08987061984b0ceb7\"><code>bb723b6</code></a>\nAdd <code>trailing_input</code> modifier to <code>end</code></li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/31c4f8e0b56e6ae24fe0d6ef0e492b6741dda783\"><code>31c4f8e</code></a>\nPermit <code>W12</code> in <code>date!</code> macro</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/490a17bf306576850f33a86d3ca95d96db7b1dcd\"><code>490a17b</code></a>\nMark error paths in well-known formats as cold</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6cb1896a600be1538ecfab8f233fe9cfe9fa8951\"><code>6cb1896</code></a>\nOptimize <code>Rfc2822</code> parsing</li>\n<li><a\nhref=\"https://github.com/time-rs/time/commit/6d264d59c25e3da0453c3defebf4640b0086a006\"><code>6d264d5</code></a>\nRemove erroneous <code>#[inline(never)]</code> attributes</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/time-rs/time/compare/v0.3.45...v0.3.47\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=time&package-manager=cargo&previous-version=0.3.45&new-version=0.3.47)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-02-05T19:18:52Z",
          "url": "https://github.com/ava-labs/firewood/commit/8a19798adf8990bb7b2de119a178867c38f9d73e"
        },
        "date": 1770709649321,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 152.19723463256815,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6570.421613862974,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.39158046190857,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6379.87692912456,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 79.75624108881817,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "99d5a3617cc4ab599fe9d82b9d0e5d93959a2b60",
          "message": "fix(track-performance): stale cron expression in daily bench (#1667)\n\n## Why this should be merged\n\nToday's scheduled daily benchmark run\n[failed](https://github.com/ava-labs/firewood/actions/runs/21894045008)\nbecause `github.event.schedule` returned `0 5 * * *` a cron expression\nthat was updated to `0 5 * * 1-5`:\nhttps://github.com/ava-labs/firewood/pull/1659\n\n\nhttps://github.com/ava-labs/firewood/actions/runs/21894045008/job/63219300944#step:2:121\n\nOpened bug report https://github.com/actions/runner/issues/4241\n\n## How this works\n- Matches the stale `0 5 * * *` cron alongside `0 5 * * 1-5` in the case\nstatement so the run doesn't hard-fail\n- Adds a weekend guard if the stale cron fires on Saturday or Sunday,\nthe run exits early since those days aren't meant for the daily\nbenchmark\n\n## How this was tested\n\nHard to test cron scheduling without trial and error approach\nunfortunately.",
          "timestamp": "2026-02-11T20:05:03Z",
          "url": "https://github.com/ava-labs/firewood/commit/99d5a3617cc4ab599fe9d82b9d0e5d93959a2b60"
        },
        "date": 1770882373624,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 140.9334935203382,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7095.545388262793,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.66889150037733,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6895.580071083648,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 84.40526332677803,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "bernard-avalabs",
            "username": "bernard-avalabs",
            "email": "53795885+bernard-avalabs@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "38c9956e940d61da8ac3c5b47ef0c5ef9f462a19",
          "message": "feat: Change proof serialization/deserialization (#1638)\n\n## Why this should be merged\n\nSecond PR for change proof FFI (builds on\nhttps://github.com/ava-labs/firewood/pull/1637). This PR includes\n`fwd_change_proof_to_bytes` and `fwd_change_proof_from_bytes` for\nserialization/deserization of a change proof.\n\n## How this works\nMostly follows the serialization/deserialization implementation in range\nproof. Main change is to support serializing/deserializing `BatchOp`s\nwhich are used for storing the difference between two revisions.\n\n## How this was tested\n\nBasic serialization and deserialization tests in `proofs_test.go`,\nincluding a round trip test where a change proof is serialized,\ndeserialized, and serialized again, and verifying the two serialized\nproofs match.",
          "timestamp": "2026-02-13T03:12:13Z",
          "url": "https://github.com/ava-labs/firewood/commit/38c9956e940d61da8ac3c5b47ef0c5ef9f462a19"
        },
        "date": 1770968735219,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 141.56204271913762,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7064.040478590884,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 113.31156801421498,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6860.831584053562,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 86.85180000599526,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "23cd1d4768f402ef7ad476dac20227f2ca84f625",
          "message": "feat: defer persist every `N` commits (#1657)\n\n## Why this should be merged\n\nExtends deferred persistence to allow for persists on every `N`th\ncommit.\n\nThis PR precedes #1650 and #1656 (in order).\n\n## How this works\n\nAdds a new configuration option `deferred_persistence_commit_count`\nwhich controls the maximum number of unpersisted revisions.\n\n  - `N = 1`: preserves current behavior (persist every commit)\n- `N > 1`: defers persistence until `N/2` commits accumulate (the\n\"sub-interval\")\n\nBelow is an example of when `deferred_persistence_commit_count = 10`:\n\n```mermaid\nsequenceDiagram                                                                                                                                                                  \n      participant Caller                                                                                                                                                           \n      participant Main as Main Thread                                                                                                                                                          \n      participant BG as Background Thread                                                                                                                                                    \n      participant Disk                                                                                                                                                             \n                                                                                                                                                                                                                                                                                                                                                                      \n      loop Commits 1-4                                                                                                                                                             \n          Caller->>Main: commit()                                                                                                                                                    \n          Main->>BG: queue revision                                                                                                                                       \n          Note right of BG: Waiting...                                                                                                                             \n      end                                                                                                                                                                          \n                                                                                                                                                                                   \n      Caller->>Main: commit() (5th)                                                                                                                                                  \n      Main->>BG: queue revision                                                                                                                                           \n      BG->>Disk: persist revision 5                                                                                                                                 \n      Note right of Disk: Sub-interval (10/2) reached                                                                                                                              \n                                                                                                                                                                                   \n      loop Commits 6-8                                                                                                                                                             \n          Caller->>Main: commit()                                                                                                                                                    \n          Main->>BG: queue revision                                                                                                                                       \n          Note right of BG: Waiting...                                                                                                                             \n      end                                                                                                                                                                          \n                                                                                                                                                                                   \n      Caller->>Main: close()                                                                                                                                                         \n      Main->>BG: shutdown + persist last committed revision                                                                                                                             \n      BG->>Disk: persist revision 8                                                                                                                                 \n      Note right of Disk: Latest committed revision is persisted        \n```\n\nOn `close()`, the last committed revision is persisted to disk.\n\n## How this was tested\n\nAdded UTs + CI.",
          "timestamp": "2026-02-13T21:00:44Z",
          "url": "https://github.com/ava-labs/firewood/commit/23cd1d4768f402ef7ad476dac20227f2ca84f625"
        },
        "date": 1771228364283,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 134.34301024262098,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7443.632520918049,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 118.84664977397887,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7233.198839920843,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 87.77499880712448,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "d401cd518934088ca8374ecf354a2e3bf2f03df8",
          "message": "chore(ffi): update CGO LDFLAGS to include -ldl (#1686)\n\nThis change adds an explicit link with `dl` to the `LDFLAGS`. If `dl` is\nultimately unused, this will have no effect.\n\n`jemalloc` has a\n[dependency](https://github.com/jemalloc/jemalloc/blob/1972241cd204c60fb5b66f23c48a117879636161/src/background_thread.c#L64-L69)\non `dlsym`. This causes the Go binary to fail to build in scenarios\nwhere `-ldl` is not otherwise inferred by the Go compiler.\n\n```text\n# github.com/ava-labs/firewood/ffi.test\n/usr/local/go-1.26.0/pkg/tool/linux_arm64/link: running gcc failed: exit status 1\n/usr/bin/gcc -s -Wl,--build-id=0x17a030a4dc77915b6306fe19d5f9f8cda10bea1b -o $WORK/b001/ffi.test -Wl,--export-dynamic-symbol=_cgo_panic -Wl,--export-dynamic-symbol=_cgo_topofstack -Wl,--export-dynamic-symbol=crosscall2 -Wl,--compress-debug-sections=zlib /tmp/go-link-1631042352/go.o /tmp/go-link-1631042352/000000.o /tmp/go-link-1631042352/000001.o /tmp/go-link-1631042352/000002.o /tmp/go-link-1631042352/000003.o /tmp/go-link-1631042352/000004.o /tmp/go-link-1631042352/000005.o /tmp/go-link-1631042352/000006.o /tmp/go-link-1631042352/000007.o /tmp/go-link-1631042352/000008.o /tmp/go-link-1631042352/000009.o /tmp/go-link-1631042352/000010.o /tmp/go-link-1631042352/000011.o /tmp/go-link-1631042352/000012.o /tmp/go-link-1631042352/000013.o /tmp/go-link-1631042352/000014.o /tmp/go-link-1631042352/000015.o /tmp/go-link-1631042352/000016.o /tmp/go-link-1631042352/000017.o /tmp/go-link-1631042352/000018.o /tmp/go-link-1631042352/000019.o /tmp/go-link-1631042352/000020.o /tmp/go-link-1631042352/000021.o /tmp/go-link-1631042352/000022.o /tmp/go-link-1631042352/000023.o /tmp/go-link-1631042352/000024.o /tmp/go-link-1631042352/000025.o /tmp/go-link-1631042352/000026.o /tmp/go-link-1631042352/000027.o /tmp/go-link-1631042352/000028.o /tmp/go-link-1631042352/000029.o /tmp/go-link-1631042352/000030.o /tmp/go-link-1631042352/000031.o /tmp/go-link-1631042352/000032.o /tmp/go-link-1631042352/000033.o -O2 -g -L/workspaces/firewood/ffi/../target/debug -L/workspaces/firewood/ffi/../target/release -L/workspaces/firewood/ffi/../target/maxperf -lfirewood_ffi -lm -O2 -g -lpthread -O2 -g -lresolv -O2 -g -no-pie\n/usr/bin/ld: /workspaces/firewood/ffi/../target/maxperf/libfirewood_ffi.a(firewood_ffi-fd0e8d1e328baab2.firewood_ffi.f340a7ea98169a93-cgu.0.rcgu.o): in function `_RNvNtNtCsd4QvH79tVyM_9getrandom8backends27linux_android_with_fallback4init':\nfirewood_ffi.f340a7ea98169a93-cgu.0:(.text.unlikely._RNvNtNtCsd4QvH79tVyM_9getrandom8backends27linux_android_with_fallback4init+0x18): undefined reference to `dlsym'\n/usr/bin/ld: /workspaces/firewood/ffi/../target/maxperf/libfirewood_ffi.a(firewood_ffi-fd0e8d1e328baab2.firewood_ffi.f340a7ea98169a93-cgu.0.rcgu.o): in function `_RNvMNtNtNtNtNtCsj8W61RwElR7_3std3sys3pal4unix4weak5dlsymINtB2_9DlsymWeakFUKCPNtNtNtNtNtNtNtCs7BotxDYo2ft_4libc4unix10linux_like5linux3gnu3b647aarch6414pthread_attr_tEjE10initializeBc_':\n/rustc/873b4beb0cc726493b94c8ef21f68795c04fbbc1/library/std/src/sys/pal/unix/weak/dlsym.rs:89: undefined reference to `dlsym'\n/usr/bin/ld: /workspaces/firewood/ffi/../target/maxperf/libfirewood_ffi.a(background_thread.pic.o): in function `pthread_create_fptr_init':\n/workspaces/firewood/target/maxperf/build/tikv-jemalloc-sys-3ae46a1d62e6e971/out/build/src/background_thread.c:724: undefined reference to `dlsym'\n/usr/bin/ld: /workspaces/firewood/target/maxperf/build/tikv-jemalloc-sys-3ae46a1d62e6e971/out/build/src/background_thread.c:724: undefined reference to `dlsym'\ncollect2: error: ld returned 1 exit status\n```\n\nFrom the missing symbols list, I also see that the rust\n[stdlib](https://github.com/rust-lang/rust/blob/71e00273c0921e1bc850ae8cc4161fbb44cfa848/library/std/src/sys/pal/unix/weak/mod.rs#L43-L47)\nhas a dependency on dlysm as well.\n\nSigned-off-by: Joachim Brandon LeBlanc <brandon.leblanc@avalabs.org>",
          "timestamp": "2026-02-16T19:48:12Z",
          "url": "https://github.com/ava-labs/firewood/commit/d401cd518934088ca8374ecf354a2e3bf2f03df8"
        },
        "date": 1771314303081,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 141.76815981841978,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7053.770051616846,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.6102310250383,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6863.508530065195,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 77.05775359136742,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "0eccc8664152d819fcc61abdba0e1ba8ab01599b",
          "message": "fix(ffi): prevent potential use-after-free in Go Iterator (#1688)\n\n## Why this should be merged\n\nFixes: #1687\n\n## How this works\n\nThis sets the handle to nil after calling free.\n\n## How this was tested\n\nAdded a test that calls `Drop` twice on an iterator which otherwise\ncaused a memory access violation.",
          "timestamp": "2026-02-17T18:48:42Z",
          "url": "https://github.com/ava-labs/firewood/commit/0eccc8664152d819fcc61abdba0e1ba8ab01599b"
        },
        "date": 1771400630959,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 145.20461461513466,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6886.83347048235,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.47650772649098,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6696.817909445412,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 76.97483233299029,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f3a60b49c33fdd9fa488cac10f626fc6dbb57d7f",
          "message": "fix: Avoid holding the last_committed_revision arc (#1685)\n\nTo reliably reproduce the original timing problem, sleep for 200ms or so\nduring the call to `self.persist`.\n\n## Why this should be merged\n\nIt's a bug.\n\n## How this works\n\nWhen persist() is slow, the persist worker holds the Arc reference\nthrough last_committed_revision for an extended period after persistence\ncompletes. This prevents the revision manager from reaping the revision\nbecause Arc::try_unwrap() fails (reference count > 1).\n\nSo, we clear the last_committed_revision the moment we've decided to\npersist it, which lowers the reference count faster.\n\n## How this was tested\n\nAdded that sleep and reproduced the bug. Added the fix and left in the\nsleep and it worked great.\n\nFixes #1684",
          "timestamp": "2026-02-18T18:57:59Z",
          "url": "https://github.com/ava-labs/firewood/commit/f3a60b49c33fdd9fa488cac10f626fc6dbb57d7f"
        },
        "date": 1771487013120,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 143.8998215045726,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6949.278946591492,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.38133411430101,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6759.81477934851,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 76.53656362710137,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f3a60b49c33fdd9fa488cac10f626fc6dbb57d7f",
          "message": "fix: Avoid holding the last_committed_revision arc (#1685)\n\nTo reliably reproduce the original timing problem, sleep for 200ms or so\nduring the call to `self.persist`.\n\n## Why this should be merged\n\nIt's a bug.\n\n## How this works\n\nWhen persist() is slow, the persist worker holds the Arc reference\nthrough last_committed_revision for an extended period after persistence\ncompletes. This prevents the revision manager from reaping the revision\nbecause Arc::try_unwrap() fails (reference count > 1).\n\nSo, we clear the last_committed_revision the moment we've decided to\npersist it, which lowers the reference count faster.\n\n## How this was tested\n\nAdded that sleep and reproduced the bug. Added the fix and left in the\nsleep and it worked great.\n\nFixes #1684",
          "timestamp": "2026-02-18T18:57:59Z",
          "url": "https://github.com/ava-labs/firewood/commit/f3a60b49c33fdd9fa488cac10f626fc6dbb57d7f"
        },
        "date": 1771573480602,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 139.27733246519438,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7179.919246729553,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 115.46928050340733,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6976.30079404668,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 84.94913761504772,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "34bc3fb596654e7d94f6df3f485390ad606a06d3",
          "message": "refactor: close consumes itself (#1701)\n\n## Why this should be merged\n\nCloses #1699.\n\n## How this works\n\nFollows the\n[RAII](https://doc.rust-lang.org/rust-by-example/scope/raii.html) design\nby having the revision manager consume itself upon closing. This\neliminates the previous scenario where, after calling `close()`, the\nrevision manager was still usable even though calls such as `commit()`\nwould fail.\n\nIn particular, this PR does the following:\n\n- Makes `RevisionManager::close()` consume itself + eliminates its\n`Drop` implementation\n- Refactors `TestDb` to automatically close the inner database at the\nend of the test\n- Adds `close()` calls to the `fwdctl` crate.\n\n## How this was tested\n\nCI.",
          "timestamp": "2026-02-20T20:18:38Z",
          "url": "https://github.com/ava-labs/firewood/commit/34bc3fb596654e7d94f6df3f485390ad606a06d3"
        },
        "date": 1771832760661,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 141.80786029253252,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7051.79528086187,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.40342972089961,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6855.640269991094,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 80.74854701193435,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "34bc3fb596654e7d94f6df3f485390ad606a06d3",
          "message": "refactor: close consumes itself (#1701)\n\n## Why this should be merged\n\nCloses #1699.\n\n## How this works\n\nFollows the\n[RAII](https://doc.rust-lang.org/rust-by-example/scope/raii.html) design\nby having the revision manager consume itself upon closing. This\neliminates the previous scenario where, after calling `close()`, the\nrevision manager was still usable even though calls such as `commit()`\nwould fail.\n\nIn particular, this PR does the following:\n\n- Makes `RevisionManager::close()` consume itself + eliminates its\n`Drop` implementation\n- Refactors `TestDb` to automatically close the inner database at the\nend of the test\n- Adds `close()` calls to the `fwdctl` crate.\n\n## How this was tested\n\nCI.",
          "timestamp": "2026-02-20T20:18:38Z",
          "url": "https://github.com/ava-labs/firewood/commit/34bc3fb596654e7d94f6df3f485390ad606a06d3"
        },
        "date": 1771919517722,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 134.24358493291047,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7449.1455252760115,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 122.50469433995,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7228.910419776427,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 93.55734741531414,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "AminR443",
            "username": "AminR443",
            "email": "amin.rezaei@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "046ce6a17a11c04ad24af9cf41f572fe96631245",
          "message": "feat(fwdctl/launch): add scenario-driven deploy, list/kill instance management, and dry-run planning (3/4) (#1664)\n\n## Why this should be merged\n\nThis PR upgrades `fwdctl launch` from deploy/monitor-only into a full\nlifecycle management tool. It adds planning (`--dry-run`), managed\ninstance operations (`list`, `kill`), and scenario-based launches.\n\n## How this works\n\nThe launch CLI now supports:\n- `fwdctl launch list` to show `ManagedBy=fwdctl` instances (with\n`--running` / `--mine`)\n- `fwdctl launch kill` to terminate a specific instance, your instances,\nor all managed instances\n- `fwdctl launch deploy --scenario <name>` to pick a scenario from\n`benchmark/launch/launch-stages.yaml`\n- `fwdctl launch deploy --dry-run [plan|plan-with-cloud-init]` to\npreview actions without creating resources\n- `fwdctl launch deploy --follow [follow|follow-with-progress]` for\nlog/progress streaming modes\n\n## How this was tested\nSome UT. mostly manually launching and ensuring all the cases work fine.\n\n## Design Notes\n\n1. `list`/`kill` are intentionally scoped to `ManagedBy=fwdctl`\nresources to avoid accidental operations on unrelated instances.\n3. `plan-with-cloud-init` was added so generated cloud-init can be\ninspected/reviewed before launch.",
          "timestamp": "2026-02-25T04:59:05Z",
          "url": "https://github.com/ava-labs/firewood/commit/046ce6a17a11c04ad24af9cf41f572fe96631245"
        },
        "date": 1772005675122,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 139.8154320700256,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7152.286304841922,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.98727760783379,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6951.693292050317,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 82.42763232466919,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "AminR443",
            "username": "AminR443",
            "email": "amin.rezaei@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "94641ecefaf724dd76e8c3d839994dbb01d368a1",
          "message": "feat(fwdctl/launch): add docs, deprecate aws-launch.sh (4/4) (#1665)\n\n## Why this should be merged\nThis PR adds necessary documentation and readme for fwdctl launch\ncommand, it also deprecates the previous `aws-launch.sh` script.\n \n## How this works\nNA.\n\n## How this was tested\nNA.",
          "timestamp": "2026-02-25T15:28:10Z",
          "url": "https://github.com/ava-labs/firewood/commit/94641ecefaf724dd76e8c3d839994dbb01d368a1"
        },
        "date": 1772048468827,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 135.94324473805395,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7356.010973012143,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 120.09476766932417,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7142.705495614944,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 89.49072776746077,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Austin Larson",
            "username": "alarso16",
            "email": "78000745+alarso16@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1a262e0f6b49f0c56ea59d43c030d7b97866ae97",
          "message": "perf: Use `runtime.AddCleanup` (#1705)\n\n## Why this should be merged\n\nIt's better practice, supposedly. At least more difficult to misuse. Fun\nfact - they can run concurrently!\n\n## How this works\n\nRequired a lot of refactoring, hopefully it's clearer. Didn't work on\nproofs yet though, not sure what the guarantees need to be\n\n## How this was tested\n\nCI",
          "timestamp": "2026-02-26T21:28:23Z",
          "url": "https://github.com/ava-labs/firewood/commit/1a262e0f6b49f0c56ea59d43c030d7b97866ae97"
        },
        "date": 1772178075351,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 142.3075485363166,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7027.034126336609,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.31989539510698,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6831.761492622895,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 80.13112364032567,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Austin Larson",
            "username": "alarso16",
            "email": "78000745+alarso16@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1a262e0f6b49f0c56ea59d43c030d7b97866ae97",
          "message": "perf: Use `runtime.AddCleanup` (#1705)\n\n## Why this should be merged\n\nIt's better practice, supposedly. At least more difficult to misuse. Fun\nfact - they can run concurrently!\n\n## How this works\n\nRequired a lot of refactoring, hopefully it's clearer. Didn't work on\nproofs yet though, not sure what the guarantees need to be\n\n## How this was tested\n\nCI",
          "timestamp": "2026-02-26T21:28:23Z",
          "url": "https://github.com/ava-labs/firewood/commit/1a262e0f6b49f0c56ea59d43c030d7b97866ae97"
        },
        "date": 1772437154404,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 142.3318373912568,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7025.834966572478,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.90712552506747,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6829.876261642077,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 80.27704119603528,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "fea4e8177d1f30a7bfbd7aaa40146b2e154c68f5",
          "message": "chore(deps): bump go.opentelemetry.io/otel/sdk from 1.22.0 to 1.40.0 in /ffi/tests/firewood (#1729)\n\nBumps\n[go.opentelemetry.io/otel/sdk](https://github.com/open-telemetry/opentelemetry-go)\nfrom 1.22.0 to 1.40.0.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/releases\">go.opentelemetry.io/otel/sdk's\nreleases</a>.</em></p>\n<blockquote>\n<h2>Release v1.23.0-rc.1</h2>\n<p>This is a release candidate for the v1.23.0 release. That release is\nexpected to include the <code>v1</code> release of the following\nmodules:</p>\n<ul>\n<li><code>go.opentelemetry.io/otel/bridge/opencensus</code></li>\n<li><code>go.opentelemetry.io/otel/bridge/opencensus/test</code></li>\n<li><code>go.opentelemetry.io/otel/example/opencensus</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetricgrpc</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/stdout/stdoutmetric</code></li>\n</ul>\n<p>See our <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/8f2bdf85ed99c6532b8c76688e7ffcf9e48c3e6d/VERSIONING.md\">versioning\npolicy</a> for more information about these stability guarantees.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/main/CHANGELOG.md\">go.opentelemetry.io/otel/sdk's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>[1.40.0/0.62.0/0.16.0] 2026-02-02</h2>\n<h3>Added</h3>\n<ul>\n<li>Add <code>AlwaysRecord</code> sampler in\n<code>go.opentelemetry.io/otel/sdk/trace</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7724\">#7724</a>)</li>\n<li>Add <code>Enabled</code> method to all synchronous instrument\ninterfaces (<code>Float64Counter</code>,\n<code>Float64UpDownCounter</code>, <code>Float64Histogram</code>,\n<code>Float64Gauge</code>, <code>Int64Counter</code>,\n<code>Int64UpDownCounter</code>, <code>Int64Histogram</code>,\n<code>Int64Gauge</code>,) in\n<code>go.opentelemetry.io/otel/metric</code>.\nThis stabilizes the synchronous instrument enabled feature, allowing\nusers to check if an instrument will process measurements before\nperforming computationally expensive operations. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7763\">#7763</a>)</li>\n<li>Add <code>go.opentelemetry.io/otel/semconv/v1.39.0</code> package.\nThe package contains semantic conventions from the <code>v1.39.0</code>\nversion of the OpenTelemetry Semantic Conventions.\nSee the <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/main/semconv/v1.39.0/MIGRATION.md\">migration\ndocumentation</a> for information on how to upgrade from\n<code>go.opentelemetry.io/otel/semconv/v1.38.0.</code> (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7783\">#7783</a>,\n<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7789\">#7789</a>)</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>Improve the concurrent performance of\n<code>HistogramReservoir</code> in\n<code>go.opentelemetry.io/otel/sdk/metric/exemplar</code> by 4x. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7443\">#7443</a>)</li>\n<li>Improve the concurrent performance of\n<code>FixedSizeReservoir</code> in\n<code>go.opentelemetry.io/otel/sdk/metric/exemplar</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7447\">#7447</a>)</li>\n<li>Improve performance of concurrent histogram measurements in\n<code>go.opentelemetry.io/otel/sdk/metric</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7474\">#7474</a>)</li>\n<li>Improve performance of concurrent synchronous gauge measurements in\n<code>go.opentelemetry.io/otel/sdk/metric</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7478\">#7478</a>)</li>\n<li>Add experimental observability metrics in\n<code>go.opentelemetry.io/otel/exporters/stdout/stdoutmetric</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7492\">#7492</a>)</li>\n<li><code>Exporter</code> in\n<code>go.opentelemetry.io/otel/exporters/prometheus</code> ignores\nmetrics with the scope\n<code>go.opentelemetry.io/contrib/bridges/prometheus</code>.\nThis prevents scrape failures when the Prometheus exporter is\nmisconfigured to get data from the Prometheus bridge. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7688\">#7688</a>)</li>\n<li>Improve performance of concurrent exponential histogram measurements\nin <code>go.opentelemetry.io/otel/sdk/metric</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7702\">#7702</a>)</li>\n<li>The <code>rpc.grpc.status_code</code> attribute in the experimental\nmetrics emitted from\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracegrpc</code>\nis replaced with the <code>rpc.response.status_code</code> attribute to\nalign with the semantic conventions. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7854\">#7854</a>)</li>\n<li>The <code>rpc.grpc.status_code</code> attribute in the experimental\nmetrics emitted from\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploggrpc</code>\nis replaced with the <code>rpc.response.status_code</code> attribute to\nalign with the semantic conventions. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7854\">#7854</a>)</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>Fix bad log message when key-value pairs are dropped because of key\nduplication in <code>go.opentelemetry.io/otel/sdk/log</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7662\">#7662</a>)</li>\n<li>Fix <code>DroppedAttributes</code> on <code>Record</code> in\n<code>go.opentelemetry.io/otel/sdk/log</code> to not count the\nnon-attribute key-value pairs dropped because of key duplication. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7662\">#7662</a>)</li>\n<li>Fix <code>SetAttributes</code> on <code>Record</code> in\n<code>go.opentelemetry.io/otel/sdk/log</code> to not log that attributes\nare dropped when they are actually not dropped. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7662\">#7662</a>)</li>\n<li>Fix missing <code>request.GetBody</code> in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp</code>\nto correctly handle HTTP/2 <code>GOAWAY</code> frame. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7794\">#7794</a>)</li>\n<li><code>WithHostID</code> detector in\n<code>go.opentelemetry.io/otel/sdk/resource</code> to use full path for\n<code>ioreg</code> command on Darwin (macOS). (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7818\">#7818</a>)</li>\n</ul>\n<h3>Deprecated</h3>\n<ul>\n<li>Deprecate <code>go.opentelemetry.io/otel/exporters/zipkin</code>.\nFor more information, see the <a\nhref=\"https://opentelemetry.io/blog/2025/deprecating-zipkin-exporters/\">OTel\nblog post deprecating the Zipkin exporter</a>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7670\">#7670</a>)</li>\n</ul>\n<h2>[1.39.0/0.61.0/0.15.0/0.0.14] 2025-12-05</h2>\n<h3>Added</h3>\n<ul>\n<li>Greatly reduce the cost of recording metrics in\n<code>go.opentelemetry.io/otel/sdk/metric</code> using hashing for map\nkeys. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7175\">#7175</a>)</li>\n<li>Add <code>WithInstrumentationAttributeSet</code> option to\n<code>go.opentelemetry.io/otel/log</code>,\n<code>go.opentelemetry.io/otel/metric</code>, and\n<code>go.opentelemetry.io/otel/trace</code> packages.\nThis provides a concurrent-safe and performant alternative to\n<code>WithInstrumentationAttributes</code> by accepting a\npre-constructed <code>attribute.Set</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7287\">#7287</a>)</li>\n<li>Add experimental observability for the Prometheus exporter in\n<code>go.opentelemetry.io/otel/exporters/prometheus</code>.\nCheck the\n<code>go.opentelemetry.io/otel/exporters/prometheus/internal/x</code>\npackage documentation for more information. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7345\">#7345</a>)</li>\n<li>Add experimental observability metrics in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploggrpc</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7353\">#7353</a>)</li>\n<li>Add temporality selector functions\n<code>DeltaTemporalitySelector</code>,\n<code>CumulativeTemporalitySelector</code>,\n<code>LowMemoryTemporalitySelector</code> to\n<code>go.opentelemetry.io/otel/sdk/metric</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7434\">#7434</a>)</li>\n<li>Add experimental observability metrics for simple log processor in\n<code>go.opentelemetry.io/otel/sdk/log</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7548\">#7548</a>)</li>\n<li>Add experimental observability metrics in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracegrpc</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7459\">#7459</a>)</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/a3a5317c5caed1656fb5b301b66dfeb3c4c944e0\"><code>a3a5317</code></a>\nRelease v1.40.0 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7859\">#7859</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/77785da545d67b38774891cbdd334368bfacdfd8\"><code>77785da</code></a>\nchore(deps): update github/codeql-action action to v4.32.1 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7858\">#7858</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/56fa1c297bf71f0ada3dbf4574a45d0607812cc0\"><code>56fa1c2</code></a>\nchore(deps): update module github.com/clipperhouse/uax29/v2 to v2.5.0\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7857\">#7857</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/298cbedf256b7a9ab3c21e41fc5e3e6d6e4e94aa\"><code>298cbed</code></a>\nUpgrade semconv use to v1.39.0 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7854\">#7854</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/3264bf171b1e6cd70f6be4a483f2bcb84eda6ccf\"><code>3264bf1</code></a>\nrefactor: modernize code (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7850\">#7850</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/fd5d030c0aa8b5bfe786299047bc914b5714d642\"><code>fd5d030</code></a>\nchore(deps): update module github.com/grpc-ecosystem/grpc-gateway/v2 to\nv2.27...</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/8d3b4cb2501dec9f1c5373123e425f109c43b8d2\"><code>8d3b4cb</code></a>\nchore(deps): update actions/cache action to v5.0.3 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7847\">#7847</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/91f7cadfcac363d67030f6913687c6dbbe086823\"><code>91f7cad</code></a>\nchore(deps): update github.com/timakin/bodyclose digest to 73d1f95 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7845\">#7845</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/fdad1eb7f350ee1f5fdb3d9a0c6855cc88ee9d75\"><code>fdad1eb</code></a>\nchore(deps): update module github.com/grpc-ecosystem/grpc-gateway/v2 to\nv2.27...</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/c46d3bac181ddaaa83286e9ccf2cd9f7705fd3d9\"><code>c46d3ba</code></a>\nchore(deps): update golang.org/x/telemetry digest to fcf36f6 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7843\">#7843</a>)</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/compare/v1.22.0...v1.40.0\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=go.opentelemetry.io/otel/sdk&package-manager=go_modules&previous-version=1.22.0&new-version=1.40.0)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-03-02T15:37:48Z",
          "url": "https://github.com/ava-labs/firewood/commit/fea4e8177d1f30a7bfbd7aaa40146b2e154c68f5"
        },
        "date": 1772523676146,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 137.86156627832256,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7253.653262441142,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 118.60518999904414,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7043.64065536172,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 87.86866395480422,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "491695b7ec9796c2a441fb33e9c7ba209faa1808",
          "message": "refactor(perf): calculate AreaIndex in as_bytes methods (#1675)\n\n## Why this should be merged\n\nRefactored Node::as_bytes and added FreeArea::as_bytes to automatically\ncalculate and return the AreaIndex instead of requiring callers to\nprovide it. This eliminates the previous double-encoding pattern where\ncallers had to encode twice to determine the correct area size.\n\n## How this works\n\n- Node::as_bytes and FreeArea::as_bytes now return a Result<AreaIndex,\nError> and calculates the area index from encoded size automatically\n- Added NodeAllocator::io_error helper method for error conversion\n- Removed double-encoding logic from serialize_node_to_bump\n- Pre-reserves exact buffer size in FreeArea::as_bytes\n- Uses AsRef<[u8]> and IndexMut trait bounds for flexibility\n- Single-pass encoding for all nodes\n\n## How this was tested\n\nTIP == test in production! Let's see if our perf numbers change after\nmerging. The unit test benchmarks won't change, and in fact may be\nslightly worse since each call is doing more work, but we do avoid\ncalling it twice in the happy path.\n\nResolves #1114",
          "timestamp": "2026-03-03T23:59:12Z",
          "url": "https://github.com/ava-labs/firewood/commit/491695b7ec9796c2a441fb33e9c7ba209faa1808"
        },
        "date": 1772610125934,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 134.49234434874552,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7435.367454127717,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 124.31665175346791,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7212.584041827687,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 94.22263817513596,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "76a2fabbcc1dfc018bf69e02360a7c3b48715843",
          "message": "ci(deps): bump the github-actions group with 3 updates (#1738)\n\nBumps the github-actions group with 3 updates:\n[actions/upload-artifact](https://github.com/actions/upload-artifact),\n[actions/download-artifact](https://github.com/actions/download-artifact)\nand\n[benchmark-action/github-action-benchmark](https://github.com/benchmark-action/github-action-benchmark).\n\nUpdates `actions/upload-artifact` from 6 to 7\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/actions/upload-artifact/releases\">actions/upload-artifact's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v7.0.0</h2>\n<h2>v7 What's new</h2>\n<h3>Direct Uploads</h3>\n<p>Adds support for uploading single files directly (unzipped). Callers\ncan set the new <code>archive</code> parameter to <code>false</code> to\nskip zipping the file during upload. Right now, we only support single\nfiles. The action will fail if the glob passed resolves to multiple\nfiles. The <code>name</code> parameter is also ignored with this\nsetting. Instead, the name of the artifact will be the name of the\nuploaded file.</p>\n<h3>ESM</h3>\n<p>To support new versions of the <code>@actions/*</code> packages,\nwe've upgraded the package to ESM.</p>\n<h2>What's Changed</h2>\n<ul>\n<li>Add proxy integration test by <a\nhref=\"https://github.com/Link\"><code>@​Link</code></a>- in <a\nhref=\"https://redirect.github.com/actions/upload-artifact/pull/754\">actions/upload-artifact#754</a></li>\n<li>Upgrade the module to ESM and bump dependencies by <a\nhref=\"https://github.com/danwkennedy\"><code>@​danwkennedy</code></a> in\n<a\nhref=\"https://redirect.github.com/actions/upload-artifact/pull/762\">actions/upload-artifact#762</a></li>\n<li>Support direct file uploads by <a\nhref=\"https://github.com/danwkennedy\"><code>@​danwkennedy</code></a> in\n<a\nhref=\"https://redirect.github.com/actions/upload-artifact/pull/764\">actions/upload-artifact#764</a></li>\n</ul>\n<h2>New Contributors</h2>\n<ul>\n<li><a href=\"https://github.com/Link\"><code>@​Link</code></a>- made\ntheir first contribution in <a\nhref=\"https://redirect.github.com/actions/upload-artifact/pull/754\">actions/upload-artifact#754</a></li>\n</ul>\n<p><strong>Full Changelog</strong>: <a\nhref=\"https://github.com/actions/upload-artifact/compare/v6...v7.0.0\">https://github.com/actions/upload-artifact/compare/v6...v7.0.0</a></p>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/actions/upload-artifact/commit/bbbca2ddaa5d8feaa63e36b76fdaad77386f024f\"><code>bbbca2d</code></a>\nSupport direct file uploads (<a\nhref=\"https://redirect.github.com/actions/upload-artifact/issues/764\">#764</a>)</li>\n<li><a\nhref=\"https://github.com/actions/upload-artifact/commit/589182c5a4cec8920b8c1bce3e2fab1c97a02296\"><code>589182c</code></a>\nUpgrade the module to ESM and bump dependencies (<a\nhref=\"https://redirect.github.com/actions/upload-artifact/issues/762\">#762</a>)</li>\n<li><a\nhref=\"https://github.com/actions/upload-artifact/commit/47309c993abb98030a35d55ef7ff34b7fa1074b5\"><code>47309c9</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/upload-artifact/issues/754\">#754</a>\nfrom actions/Link-/add-proxy-integration-tests</li>\n<li><a\nhref=\"https://github.com/actions/upload-artifact/commit/02a8460834e70dab0ce194c64360c59dc1475ef0\"><code>02a8460</code></a>\nAdd proxy integration test</li>\n<li>See full diff in <a\nhref=\"https://github.com/actions/upload-artifact/compare/v6...v7\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\nUpdates `actions/download-artifact` from 7 to 8\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/actions/download-artifact/releases\">actions/download-artifact's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v8.0.0</h2>\n<h2>v8 - What's new</h2>\n<h3>Direct downloads</h3>\n<p>To support direct uploads in <code>actions/upload-artifact</code>,\nthe action will no longer attempt to unzip all downloaded files.\nInstead, the action checks the <code>Content-Type</code> header ahead of\nunzipping and skips non-zipped files. Callers wishing to download a\nzipped file as-is can also set the new <code>skip-decompress</code>\nparameter to <code>false</code>.</p>\n<h3>Enforced checks (breaking)</h3>\n<p>A previous release introduced digest checks on the download. If a\ndownload hash didn't match the expected hash from the server, the action\nwould log a warning. Callers can now configure the behavior on mismatch\nwith the <code>digest-mismatch</code> parameter. To be secure by\ndefault, we are now defaulting the behavior to <code>error</code> which\nwill fail the workflow run.</p>\n<h3>ESM</h3>\n<p>To support new versions of the @actions/* packages, we've upgraded\nthe package to ESM.</p>\n<h2>What's Changed</h2>\n<ul>\n<li>Don't attempt to un-zip non-zipped downloads by <a\nhref=\"https://github.com/danwkennedy\"><code>@​danwkennedy</code></a> in\n<a\nhref=\"https://redirect.github.com/actions/download-artifact/pull/460\">actions/download-artifact#460</a></li>\n<li>Add a setting to specify what to do on hash mismatch and default it\nto <code>error</code> by <a\nhref=\"https://github.com/danwkennedy\"><code>@​danwkennedy</code></a> in\n<a\nhref=\"https://redirect.github.com/actions/download-artifact/pull/461\">actions/download-artifact#461</a></li>\n</ul>\n<p><strong>Full Changelog</strong>: <a\nhref=\"https://github.com/actions/download-artifact/compare/v7...v8.0.0\">https://github.com/actions/download-artifact/compare/v7...v8.0.0</a></p>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/70fc10c6e5e1ce46ad2ea6f2b72d43f7d47b13c3\"><code>70fc10c</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/download-artifact/issues/461\">#461</a>\nfrom actions/danwkennedy/digest-mismatch-behavior</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/f258da9a506b755b84a09a531814700b86ccfc62\"><code>f258da9</code></a>\nAdd change docs</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/ccc058e5fbb0bb2352213eaec3491e117cbc4a5c\"><code>ccc058e</code></a>\nFix linting issues</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/bd7976ba57ecea96e6f3df575eb922d11a12a9fd\"><code>bd7976b</code></a>\nAdd a setting to specify what to do on hash mismatch and default it to\n<code>error</code></li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/ac21fcf45e0aaee541c0f7030558bdad38d77d6c\"><code>ac21fcf</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/download-artifact/issues/460\">#460</a>\nfrom actions/danwkennedy/download-no-unzip</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/15999bff51058bc7c19b50ebbba518eaef7c26c0\"><code>15999bf</code></a>\nAdd note about package bumps</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/974686ed5098c7f9c9289ec946b9058e496a2561\"><code>974686e</code></a>\nBump the version to <code>v8</code> and add release notes</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/fbe48b1d2756394be4cd4358ed3bc1343b330e75\"><code>fbe48b1</code></a>\nUpdate test names to make it clearer what they do</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/96bf374a614d4360e225874c3efd6893a3f285e7\"><code>96bf374</code></a>\nOne more test fix</li>\n<li><a\nhref=\"https://github.com/actions/download-artifact/commit/b8c4819ef592cbe04fd93534534b38f853864332\"><code>b8c4819</code></a>\nFix skip decompress test</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/actions/download-artifact/compare/v7...v8\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\nUpdates `benchmark-action/github-action-benchmark` from 1.20.7 to 1.21.0\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases\">benchmark-action/github-action-benchmark's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v1.21.0</h2>\n<ul>\n<li><strong>fix</strong> include package name for duplicate bench names\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/330\">#330</a>)</li>\n<li><strong>fix</strong> avoid duplicate package suffix in Go benchmarks\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/337\">#337</a>)</li>\n</ul>\n<p><strong>Full Changelog</strong>: <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/compare/v1.20.7...v1.21.0\">https://github.com/benchmark-action/github-action-benchmark/compare/v1.20.7...v1.21.0</a></p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/blob/master/CHANGELOG.md\">benchmark-action/github-action-benchmark's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>Unreleased</h2>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.21.0\">v1.21.0</a>\n- 02 Mar 2026</h1>\n<ul>\n<li><strong>fix</strong> include package name for duplicate bench names\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/330\">#330</a>)</li>\n<li><strong>fix</strong> avoid duplicate package suffix in Go benchmarks\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/337\">#337</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.7\">v1.20.7</a>\n- 06 Sep 2025</h1>\n<ul>\n<li><strong>fix</strong> improve parsing for custom benchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/323\">#323</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.5\">v1.20.5</a>\n- 02 Sep 2025</h1>\n<ul>\n<li><strong>feat</strong> allow to parse generic cargo bench/criterion\nunits (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/280\">#280</a>)</li>\n<li><strong>fix</strong> add summary even when failure threshold is\nsurpassed (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/285\">#285</a>)</li>\n<li><strong>fix</strong> time units are not normalized (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/318\">#318</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.4\">v1.20.4</a>\n- 23 Oct 2024</h1>\n<ul>\n<li><strong>feat</strong> add typings and validation workflow (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/257\">#257</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.3\">v1.20.3</a>\n- 19 May 2024</h1>\n<ul>\n<li><strong>fix</strong> Catch2 v.3.5.0 changed output format (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/247\">#247</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.2\">v1.20.2</a>\n- 19 May 2024</h1>\n<ul>\n<li><strong>fix</strong> Support sub-nanosecond precision on Cargo\nbenchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/246\">#246</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.1\">v1.20.1</a>\n- 02 Apr 2024</h1>\n<ul>\n<li><strong>fix</strong> release script</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.0\">v1.20.0</a>\n- 02 Apr 2024</h1>\n<ul>\n<li><strong>fix</strong> Rust benchmarks not comparing to baseline (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/235\">#235</a>)</li>\n<li><strong>feat</strong> Comment on PR and auto update comment (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/223\">#223</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.19.3\">v1.19.3</a>\n- 02 Feb 2024</h1>\n<ul>\n<li><strong>fix</strong> ratio is NaN when previous value is 0. Now,\nprint 1 when both values are 0 and <code>+-∞</code> when divisor is 0\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/222\">#222</a>)</li>\n<li><strong>fix</strong> action hangs in some cases for go fiber\nbenchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/225\">#225</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.19.2\">v1.19.2</a>\n- 26 Jan 2024</h1>\n<ul>\n<li><strong>fix</strong> markdown rendering for summary is broken (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/218\">#218</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.19.1\">v1.19.1</a>\n- 25 Jan 2024</h1>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/commit/a7bc2366eda11037936ea57d811a43b3418d3073\"><code>a7bc236</code></a>\nrelease v1.21.0</li>\n<li>See full diff in <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/compare/4bdcce38c94cec68da58d012ac24b7b1155efe8b...a7bc2366eda11037936ea57d811a43b3418d3073\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore <dependency name> major version` will close this\ngroup update PR and stop Dependabot creating any more for the specific\ndependency's major version (unless you unignore this specific\ndependency's major version or upgrade to it yourself)\n- `@dependabot ignore <dependency name> minor version` will close this\ngroup update PR and stop Dependabot creating any more for the specific\ndependency's minor version (unless you unignore this specific\ndependency's minor version or upgrade to it yourself)\n- `@dependabot ignore <dependency name>` will close this group update PR\nand stop Dependabot creating any more for the specific dependency\n(unless you unignore this specific dependency or upgrade to it yourself)\n- `@dependabot unignore <dependency name>` will remove all of the ignore\nconditions of the specified dependency\n- `@dependabot unignore <dependency name> <ignore condition>` will\nremove the ignore condition of the specified dependency and ignore\nconditions\n\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-03-04T21:13:46Z",
          "url": "https://github.com/ava-labs/firewood/commit/76a2fabbcc1dfc018bf69e02360a7c3b48715843"
        },
        "date": 1772696173295,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 142.36653784334382,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7024.122487970961,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.73195426065591,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6823.140275659003,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 83.12890297924592,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "daf50b307dc6475f6d99d6a8098b5f0cc96753f9",
          "message": "refactor: replace `PersistSemaphore` with `PersistChannel` (#1710)\n\n## Why this should be merged\n\nQuoting #1694:\n\n> The current solution uses a combination of a modified semaphore and a\nchannel to apply back pressure, with the goal of bounding the staleness\nof the most recent persisted revision. In general, mixing semaphores and\nchannels (or any other synchronization primitive) can result in tricky\nand difficult to reason about solutions. In this case, there is an\nimplicit dependence between the values of the counter inside the\nsemaphore, and a separate counter managed by the event loop to determine\nwhen to \"release permits\" in the modified semaphore. This can lead to a\ndeadlock if the semaphore counter reaches zero before the event loop\ncounter falls below the threshold to reset the semaphore. The current\nsolution may work, but it is fragile since a change to just one of the\ncounter thresholds (max permits or count value before reset) could lead\nto a deadlock that might be difficult to find during standard tests.\n\n## How this works\n\nReplaces the `PersistSemaphore` + `crossbeam::channel` with\n`PersistChannel`, a channel-like abstraction around the locks/condvars\nmentioned in #1694. By using `PersistChannel`, we no longer have the\nissue of the `PersistWorker` and the `PersistLoop` having their own\nnotions of progress as all progress is managed via\n`PersistChannelState`.\n\nThis PR also adds a drop guard to the `PersistLoop`, such that if the\nbackground thread exits for whatever reason, the system is marked as\nshutdown which prevents the `PersistWorker` from hanging.\n\n## How this was tested\n\nCI + existing deferred persistence UTs\n\n---------\n\nCo-authored-by: Ron Kuris <ron.kuris@avalabs.org>\nCo-authored-by: Bernard Wong <bernard@avalabs.org>",
          "timestamp": "2026-03-05T19:15:29Z",
          "url": "https://github.com/ava-labs/firewood/commit/daf50b307dc6475f6d99d6a8098b5f0cc96753f9"
        },
        "date": 1772783083635,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 132.65026125673896,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7538.6206595141375,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 128.6484825084654,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7305.409148973784,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 99.86909525436602,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c443887dcebbaf46a1f8dcd59b61210e29d797ed",
          "message": "docs: clarify deferred persistence terminology (#1733)\n\n## Why this should be merged\n\nCloses #1689\n\n## How this works\n\n- Adds documentation which describes the relationship between commits,\npersists, and permits.\n- Moves mermaid diagram to `PersistWorker` documentation, enabling it's\nusage with `aquamarine`.\n\n## How this was tested\n\nCI + generated documentation for the `PersistWorker`:\n\n<img width=\"785\" height=\"1120\" alt=\"Screenshot 2026-03-05 at 14 32 11\"\nsrc=\"https://github.com/user-attachments/assets/1d41cd08-9554-4f25-b98e-557b0f1eb4fc\"\n/>\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl",
          "timestamp": "2026-03-06T11:55:15Z",
          "url": "https://github.com/ava-labs/firewood/commit/c443887dcebbaf46a1f8dcd59b61210e29d797ed"
        },
        "date": 1773042062181,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 143.1862469282421,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6983.910965283913,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.36409484571145,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6790.114701274697,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 78.564483580528,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9dad4f16b65ce3baf47c528b3cc4959a98a368f9",
          "message": "ci(changelog): items labeled 'no-changelog' are omitted from changelog (#1746)\n\n## Why this should be merged\n\nFor some features there are a bunch of PRs. We only really want the last\none to show up in the CHANGELOG. This allows us to add a label to a PR\nwhich removes it from the CHANGELOG.md when generated with cliff.\n\n## How this works\n\nConnects to github to see the PR labels now. It only makes a few\nrequests but could hit ratelimits, so logging in to github (either with\n`gh auth login` or by setting GITHUB_TOKEN) is recommended.\n\nSee\nhttps://git-cliff.org/docs/tips-and-tricks#skip-commits-by-github-pr-label\nfor more details\n\n## How this was tested\n\nThis PR doesn't show up when running `git cliff` :)\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-03-09T21:52:00Z",
          "url": "https://github.com/ava-labs/firewood/commit/9dad4f16b65ce3baf47c528b3cc4959a98a368f9"
        },
        "date": 1773128349591,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 139.6061440771589,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7163.008523803296,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 119.78531646281272,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6951.7146543577355,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 88.06358879932637,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "92ba1013c747b57aa96ade6bebe37f6f63342d2e",
          "message": "fix!: clock source miss-match between Rust and Go (#1742)\n\n## Why this should be merged\n\ncoarsetime was introduced to avoid syscall overhead from `std::time`,\nbut @demosdemon confirmed (and we verified empirically) that std::time\nuses `clock_gettime(CLOCK_MONOTONIC)` via vDSO on modern Linux which is\nresolving in userspace with no kernel entry. The two are equivalent in\ncost.\n\nRemoving coarsetime fixes the clock source mismatch between Rust\n`CLOCK_MONOTONIC_COARSE` and Go `CLOCK_MONOTONIC` that was causing the\nFirewood dashboard to show Rust commit time higher than Go commit time,\nwhich is physically impossible since Go wraps Rust synchronously via\nCGO.\n\n## How this works\n\nReplaces all `coarsetime::Instant` / `coarsetime::Duration` usages\nwith`std::time::Instant` / `std::time::Duration`\n\n## How this was tested\n\nFull methodology in #1720.\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [x] firewood-ffi (C api) — `commit_proposal` token closure parameter\ntype changes from `coarsetime::Duration` to `std::time::Duration`\n- [ ] firewood-go (Go api)\n- [ ] fwdctl",
          "timestamp": "2026-03-10T19:09:43Z",
          "url": "https://github.com/ava-labs/firewood/commit/92ba1013c747b57aa96ade6bebe37f6f63342d2e"
        },
        "date": 1773214919976,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 137.32145975187245,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7282.183001891403,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 121.90250589034592,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7064.670747091683,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 91.55603743741604,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "2d6d3c77a6719828e5a5d2bc841b98b74020470c",
          "message": "refactor!: remove v2 module (#1760)\n\n## Why this should be merged\n\nCloses #1644.\n\n## How this works\n\n- Moves `api.rs` and `batch_ops.rs` up a directory.\n- Deletes `v2` module.\n- Updates all relevant import paths.\n\n## How this was tested\n\nCI\n\n## Breaking Changes\n\n- [x] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl",
          "timestamp": "2026-03-11T20:48:12Z",
          "url": "https://github.com/ava-labs/firewood/commit/2d6d3c77a6719828e5a5d2bc841b98b74020470c"
        },
        "date": 1773301258800,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 141.483346135057,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7067.9696749992145,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.35943681945987,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6868.385276001823,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 82.13656745026881,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "857e256870ad95960a4ba8a5288b05303db67cf1",
          "message": "perf(ffi): add cgo pragma declarations (#1722)\n\n## Why this should be merged\n\nThe cgo pragma hints affect the generated cgo wrappers and slightly\nimprove the overhead of calling into firewood functions from Go.\n\n## How this works\n\n`noescape` changes the generate code from `runtime.cgoUse(param)` to\n`runtime.cgoKeepAlive(param)`. This change keeps the param alive for the\nduration of the cgo call without triggering GC escape. `nocallback`\ninforms the runtime that the cgo call will not make any reentrant calls\nto the go runtime.\n\n## How this was tested\n\nCI and repeated testing during stack analysis.",
          "timestamp": "2026-03-12T19:03:51Z",
          "url": "https://github.com/ava-labs/firewood/commit/857e256870ad95960a4ba8a5288b05303db67cf1"
        },
        "date": 1773387588122,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 140.90580596678998,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7096.9396409094015,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.11894969983582,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6897.7704815941515,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 81.99601538134611,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "5406b6858e002d783b5a9c32db9b49fd18291c0b",
          "message": "chore(revert): remove nix flake and all references to it (#1788) (#1794)\n\n## Why this should be merged\n\nThe nix flake is still required by the AvalancheGo `polyrepo` build\ntool. Removing it breaks polyrepo which relies on the flake to build\nFirewood and set up the testing infrastructure used for scheduled daily\nbenchmarks, experiments, and A/B testing.\n\nThis revert restores that functionality until the migration to\nAvalancheGo is complete and the flake can be safely removed as part of\nthat transition.\n\n## How this works\n\nThis reverts commit 76a2c8c17e640604ce82d11b5a2779c9dbdd7c04.\n\n## How this was tested\n```\nTEST=firewood-101-250k FIREWOOD_REF=es/revert-remove-nix-flake just bench-cchain\n```\n- Proves just bench-cchain works\n- Successful build at FIREWOOD_REF\nhttps://github.com/ava-labs/avalanchego/actions/runs/23064572050/job/66999562936#step:5:257\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl\n\n---------\n\nCo-authored-by: maru <maru.newby@avalabs.org>",
          "timestamp": "2026-03-14T12:43:41Z",
          "url": "https://github.com/ava-labs/firewood/commit/5406b6858e002d783b5a9c32db9b49fd18291c0b"
        },
        "date": 1773647390824,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 145.74713159162602,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6861.198495500653,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.13825570971645,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6672.781373033463,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 75.80448992550646,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "5406b6858e002d783b5a9c32db9b49fd18291c0b",
          "message": "chore(revert): remove nix flake and all references to it (#1788) (#1794)\n\n## Why this should be merged\n\nThe nix flake is still required by the AvalancheGo `polyrepo` build\ntool. Removing it breaks polyrepo which relies on the flake to build\nFirewood and set up the testing infrastructure used for scheduled daily\nbenchmarks, experiments, and A/B testing.\n\nThis revert restores that functionality until the migration to\nAvalancheGo is complete and the flake can be safely removed as part of\nthat transition.\n\n## How this works\n\nThis reverts commit 76a2c8c17e640604ce82d11b5a2779c9dbdd7c04.\n\n## How this was tested\n```\nTEST=firewood-101-250k FIREWOOD_REF=es/revert-remove-nix-flake just bench-cchain\n```\n- Proves just bench-cchain works\n- Successful build at FIREWOOD_REF\nhttps://github.com/ava-labs/avalanchego/actions/runs/23064572050/job/66999562936#step:5:257\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl\n\n---------\n\nCo-authored-by: maru <maru.newby@avalabs.org>",
          "timestamp": "2026-03-14T12:43:41Z",
          "url": "https://github.com/ava-labs/firewood/commit/5406b6858e002d783b5a9c32db9b49fd18291c0b"
        },
        "date": 1773733817091,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 136.3156519549236,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 7335.914736560676,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 118.58175412111264,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7126.786234954631,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 87.11596498120346,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "a53c7e930a81a50355ed6198bdaa921bb0284308",
          "message": "chore(deps): bump lz4_flex from 0.11.5 to 0.11.6 (#1799)\n\nBumps [lz4_flex](https://github.com/pseitz/lz4_flex) from 0.11.5 to\n0.11.6.\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/PSeitz/lz4_flex/blob/main/CHANGELOG.md\">lz4_flex's\nchangelog</a>.</em></p>\n<blockquote>\n<h1>0.11.6 (2026-03-14)</h1>\n<h3>Security Fix</h3>\n<ul>\n<li>Fix handling of invalid match offsets during decompression <a\nhref=\"https://github.com/PSeitz/lz4_flex/commit/84cdafb\">#84cdafb</a>\n(thanks <a\nhref=\"https://github.com/Marcono1234\"><code>@​Marcono1234</code></a>)</li>\n</ul>\n<pre><code>Invalid match offsets (offset == 0) during decompression were\nnot properly\nhandled, which could lead to invalid memory reads on untrusted input.\nUsers on 0.11.x should upgrade to 0.11.6.\n</code></pre>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/PSeitz/lz4_flex/commit/6460047c0ba18bf4e3331894c8db220bc724a439\"><code>6460047</code></a>\nbump version to 0.11.6</li>\n<li><a\nhref=\"https://github.com/PSeitz/lz4_flex/commit/84cdafba1fb00313b6da8fd7b3cdeaf8ad07e11a\"><code>84cdafb</code></a>\nfix handling of invalid match offsets during decompression</li>\n<li>See full diff in <a\nhref=\"https://github.com/pseitz/lz4_flex/compare/0.11.5...0.11.6\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=lz4_flex&package-manager=cargo&previous-version=0.11.5&new-version=0.11.6)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-03-17T21:25:38Z",
          "url": "https://github.com/ava-labs/firewood/commit/a53c7e930a81a50355ed6198bdaa921bb0284308"
        },
        "date": 1773819901511,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 144.96661772994514,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6898.139831494697,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.53993425977566,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6704.937021521259,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 78.08319878161042,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f912a2ca6c6c0efd151d351ab3af766b4d781caa",
          "message": "chore(deps): bump google.golang.org/grpc from 1.66.0 to 1.79.3 in /ffi/tests/firewood (#1815)\n\nBumps [google.golang.org/grpc](https://github.com/grpc/grpc-go) from\n1.66.0 to 1.79.3.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/grpc/grpc-go/releases\">google.golang.org/grpc's\nreleases</a>.</em></p>\n<blockquote>\n<h2>Release 1.79.3</h2>\n<h1>Security</h1>\n<ul>\n<li>server: fix an authorization bypass where malformed :path headers\n(missing the leading slash) could bypass path-based restricted\n&quot;deny&quot; rules in interceptors like <code>grpc/authz</code>. Any\nrequest with a non-canonical path is now immediately rejected with an\n<code>Unimplemented</code> error. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8981\">#8981</a>)</li>\n</ul>\n<h2>Release 1.79.2</h2>\n<h1>Bug Fixes</h1>\n<ul>\n<li>stats: Prevent redundant error logging in health/ORCA producers by\nskipping stats/tracing processing when no stats handler is configured.\n(<a\nhref=\"https://redirect.github.com/grpc/grpc-go/pull/8874\">grpc/grpc-go#8874</a>)</li>\n</ul>\n<h2>Release 1.79.1</h2>\n<h1>Bug Fixes</h1>\n<ul>\n<li>grpc: Remove the <code>-dev</code> suffix from the User-Agent\nheader. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/pull/8902\">grpc/grpc-go#8902</a>)</li>\n</ul>\n<h2>Release 1.79.0</h2>\n<h1>API Changes</h1>\n<ul>\n<li>mem: Add experimental API <code>SetDefaultBufferPool</code> to\nchange the default buffer pool. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8806\">#8806</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/vanja-p\"><code>@​vanja-p</code></a></li>\n</ul>\n</li>\n<li>experimental/stats: Update <code>MetricsRecorder</code> to require\nembedding the new <code>UnimplementedMetricsRecorder</code> (a no-op\nstruct) in all implementations for forward compatibility. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8780\">#8780</a>)</li>\n</ul>\n<h1>Behavior Changes</h1>\n<ul>\n<li>balancer/weightedtarget: Remove handling of <code>Addresses</code>\nand only handle <code>Endpoints</code> in resolver updates. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8841\">#8841</a>)</li>\n</ul>\n<h1>New Features</h1>\n<ul>\n<li>experimental/stats: Add support for asynchronous gauge metrics\nthrough the new <code>AsyncMetricReporter</code> and\n<code>RegisterAsyncReporter</code> APIs. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8780\">#8780</a>)</li>\n<li>pickfirst: Add support for weighted random shuffling of endpoints,\nas described in <a\nhref=\"https://redirect.github.com/grpc/proposal/pull/535\">gRFC A113</a>.\n<ul>\n<li>This is enabled by default, and can be turned off using the\nenvironment variable\n<code>GRPC_EXPERIMENTAL_PF_WEIGHTED_SHUFFLING</code>. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8864\">#8864</a>)</li>\n</ul>\n</li>\n<li>xds: Implement <code>:authority</code> rewriting, as specified in <a\nhref=\"https://github.com/grpc/proposal/blob/master/A81-xds-authority-rewriting.md\">gRFC\nA81</a>. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8779\">#8779</a>)</li>\n<li>balancer/randomsubsetting: Implement the\n<code>random_subsetting</code> LB policy, as specified in <a\nhref=\"https://github.com/grpc/proposal/blob/master/A68-random-subsetting.md\">gRFC\nA68</a>. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8650\">#8650</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/marek-szews\"><code>@​marek-szews</code></a></li>\n</ul>\n</li>\n</ul>\n<h1>Bug Fixes</h1>\n<ul>\n<li>credentials/tls: Fix a bug where the port was not stripped from the\nauthority override before validation. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8726\">#8726</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/Atul1710\"><code>@​Atul1710</code></a></li>\n</ul>\n</li>\n<li>xds/priority: Fix a bug causing delayed failover to lower-priority\nclusters when a higher-priority cluster is stuck in\n<code>CONNECTING</code> state. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8813\">#8813</a>)</li>\n<li>health: Fix a bug where health checks failed for clients using\nlegacy compression options (<code>WithDecompressor</code> or\n<code>RPCDecompressor</code>). (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8765\">#8765</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/sanki92\"><code>@​sanki92</code></a></li>\n</ul>\n</li>\n<li>transport: Fix an issue where the HTTP/2 server could skip header\nsize checks when terminating a stream early. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8769\">#8769</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/joybestourous\"><code>@​joybestourous</code></a></li>\n</ul>\n</li>\n<li>server: Propagate status detail headers, if available, when\nterminating a stream during request header processing. (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8754\">#8754</a>)\n<ul>\n<li>Special Thanks: <a\nhref=\"https://github.com/joybestourous\"><code>@​joybestourous</code></a></li>\n</ul>\n</li>\n</ul>\n<h1>Performance Improvements</h1>\n<ul>\n<li>credentials/alts: Optimize read buffer alignment to reduce copies.\n(<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8791\">#8791</a>)</li>\n<li>mem: Optimize pooling and creation of <code>buffer</code> objects.\n(<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8784\">#8784</a>)</li>\n<li>transport: Reduce slice re-allocations by reserving slice capacity.\n(<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8797\">#8797</a>)</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/dda86dbd9cecb8b35b58c73d507d81d67761205f\"><code>dda86db</code></a>\nChange version to 1.79.3 (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8983\">#8983</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/72186f163e75a065c39e6f7df9b6dea07fbdeff5\"><code>72186f1</code></a>\ngrpc: enforce strict path checking for incoming requests on the server\n(<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8981\">#8981</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/97ca3522b239edf6813e2b1106924e9d55e89d43\"><code>97ca352</code></a>\nChanging version to 1.79.3-dev (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8954\">#8954</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/8902ab6efea590f5b3861126559eaa26fa9783b2\"><code>8902ab6</code></a>\nChange the version to release 1.79.2 (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8947\">#8947</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/a9286705aa689bee321ec674323b6896284f3e02\"><code>a928670</code></a>\nCherry-pick <a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8874\">#8874</a> to\nv1.79.x (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8904\">#8904</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/06df3638c0bcee88197b1033b3ba83e1eb8bc010\"><code>06df363</code></a>\nChange version to 1.79.2-dev (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8903\">#8903</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/782f2de44f597af18a120527e7682a6670d84289\"><code>782f2de</code></a>\nChange version to 1.79.1 (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8902\">#8902</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/850eccbb2257bd2de6ac28ee88a7172ab6175629\"><code>850eccb</code></a>\nChange version to 1.79.1-dev (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8851\">#8851</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/765ff056b6890f6c8341894df4e9668e9bfc18ef\"><code>765ff05</code></a>\nChange version to 1.79.0 (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8850\">#8850</a>)</li>\n<li><a\nhref=\"https://github.com/grpc/grpc-go/commit/68804be0e78ed0365bb5a576dedc12e2168ed63e\"><code>68804be</code></a>\nCherry pick <a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8864\">#8864</a> to\nv1.79.x (<a\nhref=\"https://redirect.github.com/grpc/grpc-go/issues/8896\">#8896</a>)</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/grpc/grpc-go/compare/v1.66.0...v1.79.3\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=google.golang.org/grpc&package-manager=go_modules&previous-version=1.66.0&new-version=1.79.3)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-03-19T00:46:07Z",
          "url": "https://github.com/ava-labs/firewood/commit/f912a2ca6c6c0efd151d351ab3af766b4d781caa"
        },
        "date": 1773905255165,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 169.95309866868214,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5883.976272474246,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 109.96768406071453,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5694.7962593508555,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 76.63201476677136,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "b76315ebf2a0bcbdbd0714bf656ef487476ae858",
          "message": "feat(reconstruction): Add Go bindings and tests (#1814)\n\n## Why this should be merged\n\nAdds a go layer for reconstructed revisions\n\n## How this works\n\nAdded reconstructed.go with Go wrapper for ReconstructedHandle\n\n## How this was tested\n\nAdded reconstructed_test.go with comprehensive tests\n\n## Breaking Changes\n\nNone, but adds new functionality\n\n---------\n\nSigned-off-by: Ron Kuris <swcafe@gmail.com>\nCo-authored-by: Austin Larson <78000745+alarso16@users.noreply.github.com>",
          "timestamp": "2026-03-19T22:05:01Z",
          "url": "https://github.com/ava-labs/firewood/commit/b76315ebf2a0bcbdbd0714bf656ef487476ae858"
        },
        "date": 1773991347265,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 168.11924625994365,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5948.1589541141175,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.20919282691123,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5758.390971285127,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 77.00834768716531,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "3da98c89bbd5912b22538f7a5ff6dbc488913f0e",
          "message": "chore(ci): Pedantically check all golang errors (#1820)\n\n## Why this should be merged\n\nLots of churn in https://github.com/ava-labs/firewood/pull/1814 over\nchecking the return value of a function that cannot error.\n\nThis was flagged by:\n- github copilot's review\nhttps://github.com/ava-labs/firewood/pull/1814#discussion_r2961404379\n- @maru-ava\nhttps://github.com/ava-labs/firewood/pull/1814#discussion_r2961496332\n- @RodrigoVillar\nhttps://github.com/ava-labs/firewood/pull/1814#discussion_r2962086982\n\nand required 3 changes to meet all reviewers concerns.\n\n## How this works\n\nThis catches the error at lint time, and prohibits check-ins that don't\ncheck errors on everything in the exclude list. For the list of\nexclusions, that are no longer allowed, check out the source for\nerrcheck here\nhttps://github.com/kisielk/errcheck/blob/master/errcheck/excludes.go or\nopen the thingy below:\n\n<details><summary>errcheck excludes</summary>\n<code>\n\t// bytes\n\t\"(*bytes.Buffer).Write\",\n\t\"(*bytes.Buffer).WriteByte\",\n\t\"(*bytes.Buffer).WriteRune\",\n\t\"(*bytes.Buffer).WriteString\",\n\n\t// crypto\n\t\"crypto/rand.Read\", // https://github.com/golang/go/issues/66821\n\n\t// fmt\n\t\"fmt.Print\",\n\t\"fmt.Printf\",\n\t\"fmt.Println\",\n\t\"fmt.Fprint(*bytes.Buffer)\",\n\t\"fmt.Fprintf(*bytes.Buffer)\",\n\t\"fmt.Fprintln(*bytes.Buffer)\",\n\t\"fmt.Fprint(*strings.Builder)\",\n\t\"fmt.Fprintf(*strings.Builder)\",\n\t\"fmt.Fprintln(*strings.Builder)\",\n\t\"fmt.Fprint(os.Stderr)\",\n\t\"fmt.Fprintf(os.Stderr)\",\n\t\"fmt.Fprintln(os.Stderr)\",\n\n\t// io\n\t\"(*io.PipeReader).CloseWithError\",\n\t\"(*io.PipeWriter).CloseWithError\",\n\n\t// math/rand\n\t\"math/rand.Read\",\n\t\"(*math/rand.Rand).Read\",\n\n\t// strings\n\t\"(*strings.Builder).Write\",\n\t\"(*strings.Builder).WriteByte\",\n\t\"(*strings.Builder).WriteRune\",\n\t\"(*strings.Builder).WriteString\",\n\n\t// hash\n\t\"(hash.Hash).Write\",\n\n\t// hash/maphash\n\t\"(*hash/maphash.Hash).Write\",\n\t\"(*hash/maphash.Hash).WriteByte\",\n\t\"(*hash/maphash.Hash).WriteString\",\n</code>\n</details> \n\n## How this was tested\n\nChanged the code to just call rng.Read and the linter failed\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-03-20T20:06:43Z",
          "url": "https://github.com/ava-labs/firewood/commit/3da98c89bbd5912b22538f7a5ff6dbc488913f0e"
        },
        "date": 1774251260602,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 161.45131249984837,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6193.8177182730515,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 119.82697407047031,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5980.294530146512,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 89.78313451924166,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "21cf4ec1bdb1f4b4a3395a8463f2991877280295",
          "message": "fix(deferred-persistence): carry metrics context into worker (#1829)\n\n## Why this should be merged\n\nWhen discussing the metrics context, I had an urge to check if the\ndeferred persistence worker was carrying the metrics contexts into its\nthread and discovered that it wasn't.\n\n## How this works\n\nSimilar to the parallel hash workers, the thread context is applied\nafter spawning the thread.\n\n\nhttps://github.com/ava-labs/firewood/blob/b45305cc14ccb79f509fb735d7e8bb70af39c0c1/firewood/src/merkle/parallel.rs#L262-L265\n\n## How this was tested\n\nThe added tests verifies that carrying a context into the thread\nactually does something, but doesn't explicitly test that this thread\ndoes something. Testing that is a little more complicated and I'll\nfigure that out as needed if this becomes a pattern.\n\n## Breaking Changes\n\nn/a",
          "timestamp": "2026-03-24T00:03:35Z",
          "url": "https://github.com/ava-labs/firewood/commit/21cf4ec1bdb1f4b4a3395a8463f2991877280295"
        },
        "date": 1774337254134,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 168.27616150914926,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5942.61237617801,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 111.68664637452076,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5749.285857732777,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 78.85624418444576,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "04e1222398b3cebf04f5183f1958f8349bf9071b",
          "message": "chore(deps): update rust deps (#1830)",
          "timestamp": "2026-03-24T21:47:03Z",
          "url": "https://github.com/ava-labs/firewood/commit/04e1222398b3cebf04f5183f1958f8349bf9071b"
        },
        "date": 1774423830703,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 164.54396660280545,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6077.403022706457,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.31594351494553,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5877.774451154677,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 82.1660907055551,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4fb8e85fee6d6b6cbf347f497b41200767b53ea6",
          "message": "refactor(merkle): 3/6 fix dup detection in dump_node with keep_alive vec (#1832)\n\n## Why this should be merged\n\nDiscovered this problem while debugging merkle tries. Now that\nreconstructed revisions often contain Node types, the address of the\nNode Arc is used as a unique identifier. Unfortunately, these are reused\nwhile walking a reconstructed node, leading to false \"dup\" reports and\nnot actually traversing the children (to avoid infinite loops).\n\n## How this works\n\nKeep created nodes around to give them unique addresses. This uses more\nmemory but we typically never dump anything too big.\n\n## How this was tested\n\nUsed during debugging and it seems to work. I'm not sure I want a test\naround this because it's primarily a debugging tool, but if you feel\nstrongly that we should test it speak up.\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-03-25T21:17:43Z",
          "url": "https://github.com/ava-labs/firewood/commit/4fb8e85fee6d6b6cbf347f497b41200767b53ea6"
        },
        "date": 1774510281515,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 169.39199186529194,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5903.466799039972,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.74799609533032,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5717.49145844539,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 74.91957190092782,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c4fb05ff2e76a1029005d8a22f5757c5006069d1",
          "message": "test(proofs): 5/6 enable ignored range proof tests and add negative tests (#1834)\n\n## Why this should be merged\n\nUn-ignores all 14 range proof tests that were blocked on issue #738\n(full trie reconstruction). These tests now pass with the verification\nlogic from the previous commit.\n\nAdds three negative tests that construct tampered RangeProofs with valid\nedge proofs and assert that verify_range_proof rejects them:\n- test_bad_range_proof_modified_key: flips a bit in a middle key\n- test_bad_range_proof_modified_value: flips a bit in a middle value\n- test_bad_range_proof_gapped_entries: removes a middle entry\n\nFuzz testing is in a later commit.\n\n## How this works\n\nnew tests\n\n## How this was tested\n\nCI\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-03-26T23:55:32Z",
          "url": "https://github.com/ava-labs/firewood/commit/c4fb05ff2e76a1029005d8a22f5757c5006069d1"
        },
        "date": 1774596824422,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 168.9870240062285,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5917.614123810727,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 111.32043306164152,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5725.117918058455,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 78.40654725892767,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4a6fa2857b3963f387e87b498453131e5ad1db1f",
          "message": "refactor(proofs): replace ChildMask [bool; 16] with u16 newtype (#1849)\n\n## Why this should be merged\n\nA ChildMask is much cleaner as a newtype and using bit manipulations.\nCompiler explorer shows that most of this reduces to a few bit\noperations.\n\nAlso verify_range_proof became enormous, so it was split up in this diff\nas well. Just created verify_proof_node_values and verify_root_hash as\nhelpers.\n\nAlso, added some unit tests for outside_children. This allowed for some\nrefactoring and testing at a higher level to detect problems a bit\nearlier.\n\n## How this was tested\n\nCI\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-03-27T20:18:12Z",
          "url": "https://github.com/ava-labs/firewood/commit/4a6fa2857b3963f387e87b498453131e5ad1db1f"
        },
        "date": 1774856477731,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 170.25478733136984,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5873.549963994156,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.39209908504655,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5687.224214310081,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 75.58842641106857,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "34819ab6a0bdb4e718dd2a37967cce2fa420350b",
          "message": "refactor(merkle)!: reduce visibility of firewood::merkle module (#1858)\n\n## Why this should be merged\n\nfirewood::Merkle keeps bleeding into ffi, and it should not.\n\n## How this works\n\nThis reduces the visibility by adding a test_utils feature flag. Some\nthings became unreachable as they were dead code.\n\n## How this was tested\n\nCI\n\n## Breaking Changes\n\n- [X] firewood\n\nTechnically this breaks the firewood interface, as it removes Merkle,\nbut nobody should have been relying on it.\n\n---------\n\nSigned-off-by: Ron Kuris <swcafe@gmail.com>\nCo-authored-by: Joachim Brandon LeBlanc <brandon.leblanc@avalabs.org>",
          "timestamp": "2026-03-30T21:10:57Z",
          "url": "https://github.com/ava-labs/firewood/commit/34819ab6a0bdb4e718dd2a37967cce2fa420350b"
        },
        "date": 1774942167415,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 174.42633447034368,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5733.079256848238,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 106.30631517260939,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5552.0395033347095,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 72.60359587935076,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "592cefc4231fd4fd1572903aacef7f4ed2a74fe5",
          "message": "feat(metrics): add firewood_describe_counter! and firewood_describe_gauge! facades (#1853)\n\n## Why this should be merged\nRoute all metric descriptions through firewood_metrics so registry\nmodules no longer import metrics directly. The firewood_ namespace\nprefix will be applied here in a follow-up PR.\n\nMetric names are constrained to $name:literal so that concat! can fuse\nthe prefix at compile time and the metrics crate can cache the key in a\nper-callsite static, avoiding a new allocation on every call.\n\n## How this works\nAdd two facades over `metrics::describe_counter!` and\n`metrics::describe_gauge!`.\n\nConstraining `$name` to `:literal` means the `metrics` crate's\n`key_var!` macro takes its `$name:literal` arm, which stores the key in\na `static` at the callsite rather than constructing a new `Key` on every\ncall.\n\n## How this was tested\n`cargo check -p firewood-ffi --features ethhash,logger`\n\n## Breaking Changes\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl",
          "timestamp": "2026-03-31T18:58:07Z",
          "url": "https://github.com/ava-labs/firewood/commit/592cefc4231fd4fd1572903aacef7f4ed2a74fe5"
        },
        "date": 1775029289875,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 165.50487748507524,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6042.118003985574,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 113.41935046008282,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5844.460476353633,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 81.30026652170031,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "25990da8a1407b6f91cba2a22180e73651dbe3ae",
          "message": "ci(deps): bump the github-actions group with 4 updates (#1871)\n\nBumps the github-actions group with 4 updates:\n[DavidAnson/markdownlint-cli2-action](https://github.com/davidanson/markdownlint-cli2-action),\n[DeterminateSystems/nix-installer-action](https://github.com/determinatesystems/nix-installer-action),\n[actions/configure-pages](https://github.com/actions/configure-pages)\nand\n[benchmark-action/github-action-benchmark](https://github.com/benchmark-action/github-action-benchmark).\n\nUpdates `DavidAnson/markdownlint-cli2-action` from 22.0.0 to 23.0.0\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/ce4853d43830c74c1753b39f3cf40f71c2031eb9\"><code>ce4853d</code></a>\nUpdate to version 23.0.0.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/63a898cb49b76a26a0a3f70530e6917c15353072\"><code>63a898c</code></a>\nImprove type fidelity.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/08fc3a21b172038517c83572cf786048cf458ed0\"><code>08fc3a2</code></a>\nAdd configPointer input, examples for package.json/pyproject.toml.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/154744fb7a91a75fa504780dd1118f262f1f00b5\"><code>154744f</code></a>\nFreshen generated index.js file.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/d1d523c8944fd078d27353eee558adc4207e1230\"><code>d1d523c</code></a>\nBump markdownlint-cli2 from 0.21.0 to 0.22.0</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/619b235bab3aabc522961c6d1c52632dd2938a61\"><code>619b235</code></a>\nBump eslint from 10.0.3 to 10.1.0</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/a226cbe56117f41439eb9ec69b4d404eaddd491f\"><code>a226cbe</code></a>\nFreshen generated index.js file.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/5d93b2e51956882fac0ff6308c0e3fba8fdfe65c\"><code>5d93b2e</code></a>\nMigrate from Node.js 20 to Node.js 24.</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/0cf8cddd9e0066bdb1f84b7a75b2576f53f6854a\"><code>0cf8cdd</code></a>\nBump eslint from 10.0.2 to 10.0.3</li>\n<li><a\nhref=\"https://github.com/DavidAnson/markdownlint-cli2-action/commit/462cc85e3cf3da90c9c8cb57e485ddfb614bcb66\"><code>462cc85</code></a>\nBump <code>@​stylistic/eslint-plugin</code> from 5.9.0 to 5.10.0</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/davidanson/markdownlint-cli2-action/compare/07035fd053f7be764496c0f8d8f9f41f98305101...ce4853d43830c74c1753b39f3cf40f71c2031eb9\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\nUpdates `DeterminateSystems/nix-installer-action` from 21 to 22\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/determinatesystems/nix-installer-action/releases\">DeterminateSystems/nix-installer-action's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v22</h2>\n<h2>What's Changed</h2>\n<ul>\n<li>Update <code>detsys-ts</code>: Merge pull request <a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/116\">#116</a>\nfrom\nDeterminateSystems/dependabot/github_actions/actions-deps-76468cb07f by\n<a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/211\">DeterminateSystems/nix-installer-action#211</a></li>\n<li>Update <code>detsys-ts</code>: Update main and types fields in\npackage.json (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/119\">#119</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/212\">DeterminateSystems/nix-installer-action#212</a></li>\n<li>Provide Determinate Nix vs. upstream Nix instructions by <a\nhref=\"https://github.com/lucperkins\"><code>@​lucperkins</code></a> in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/210\">DeterminateSystems/nix-installer-action#210</a></li>\n<li>Update <code>detsys-ts</code>: Merge pull request <a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/126\">#126</a>\nfrom DeterminateSystems/dependabot/npm_and_yarn/npm-deps-939209f320 by\n<a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/220\">DeterminateSystems/nix-installer-action#220</a></li>\n<li>Add summary toggle option by <a\nhref=\"https://github.com/andre4ik3\"><code>@​andre4ik3</code></a> in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/217\">DeterminateSystems/nix-installer-action#217</a></li>\n<li>Tidy up the macos runner list by <a\nhref=\"https://github.com/grahamc\"><code>@​grahamc</code></a> in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/224\">DeterminateSystems/nix-installer-action#224</a></li>\n<li>Update <code>detsys-ts</code>: Bumps (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/131\">#131</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/223\">DeterminateSystems/nix-installer-action#223</a></li>\n<li>Update <code>detsys-ts</code>: Bump fast-xml-parser from 5.3.3 to\n5.3.4 (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/134\">#134</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/228\">DeterminateSystems/nix-installer-action#228</a></li>\n<li>Update <code>detsys-ts</code>: Bump the npm-deps group across 1\ndirectory with 9 updates (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/138\">#138</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/230\">DeterminateSystems/nix-installer-action#230</a></li>\n<li>Update <code>detsys-ts</code>: Bump fast-xml-parser from 5.3.4 to\n5.3.6 (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/140\">#140</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/231\">DeterminateSystems/nix-installer-action#231</a></li>\n<li>Update <code>detsys-ts</code>: Fix default value for Action option\n(<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/144\">#144</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/233\">DeterminateSystems/nix-installer-action#233</a></li>\n<li>Update <code>detsys-ts</code>: unoptional timeout (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/146\">#146</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/235\">DeterminateSystems/nix-installer-action#235</a></li>\n<li>Attach build provenance by <a\nhref=\"https://github.com/grahamc\"><code>@​grahamc</code></a> in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/236\">DeterminateSystems/nix-installer-action#236</a></li>\n<li>Update <code>detsys-ts</code>: Drop the old schemas and integrate\nthe open PRs (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/162\">#162</a>)\nby <a\nhref=\"https://github.com/detsys-pr-bot\"><code>@​detsys-pr-bot</code></a>\nin <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/238\">DeterminateSystems/nix-installer-action#238</a></li>\n<li>Update deps, go to node24 by <a\nhref=\"https://github.com/grahamc\"><code>@​grahamc</code></a> in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/239\">DeterminateSystems/nix-installer-action#239</a></li>\n</ul>\n<h2>New Contributors</h2>\n<ul>\n<li><a href=\"https://github.com/andre4ik3\"><code>@​andre4ik3</code></a>\nmade their first contribution in <a\nhref=\"https://redirect.github.com/DeterminateSystems/nix-installer-action/pull/217\">DeterminateSystems/nix-installer-action#217</a></li>\n</ul>\n<p><strong>Full Changelog</strong>: <a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/compare/v21...v22\">https://github.com/DeterminateSystems/nix-installer-action/compare/v21...v22</a></p>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/ef8a148080ab6020fd15196c2084a2eea5ff2d25\"><code>ef8a148</code></a>\nUpdate deps, go to node24 (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/239\">#239</a>)</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/e02dcf858cd1ca353a03986f1f6dc39ff9aa5ba2\"><code>e02dcf8</code></a>\nUpdate <code>detsys-ts</code>: Drop the old schemas and integrate the\nopen PRs (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/162\">#162</a>)\n(#...</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/9a59e15a74545c99a626ba594edcd0d02189e670\"><code>9a59e15</code></a>\nAttach build provenance (<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/236\">#236</a>)</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/d96bc962e61b3049ce8128d03d57a1144fa96539\"><code>d96bc96</code></a>\nUpdate <code>detsys-ts</code> for: <code>unoptional timeout\n([#146](https://github.com/determinatesystems/nix-installer-action/issues/146))</code>\n(`a621ba724bb21cc2907e525...</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/874a9842e1ba38da0fc8c32421672a81e9843295\"><code>874a984</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/233\">#233</a>\nfrom detsys-pr-bot/detsys-ts-update-d0fa3dbd59ce2872d...</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/1ae25535ec10252a19af344ef0f648e9ecb1c506\"><code>1ae2553</code></a>\nUpdate <code>detsys-ts</code> for: <code>Fix default value for Action\noption\n([#144](https://github.com/determinatesystems/nix-installer-action/issues/144))</code>\n(`d0fa3d...</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/d9137d7b28da568ff80d73130b026d655de5508d\"><code>d9137d7</code></a>\nflake.lock: Update</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/95f009f8cba987d36d7e3396d29de81b2883654a\"><code>95f009f</code></a>\nUpdate <code>detsys-ts</code>: Bump fast-xml-parser from 5.3.4 to 5.3.6\n(<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/140\">#140</a>)\n(<a\nhref=\"https://redirect.github.com/determinatesystems/nix-installer-action/issues/231\">#231</a>)</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/86cbc893b3c44423bb3b27da33c306deba1e1ef4\"><code>86cbc89</code></a>\nUpdate <code>detsys-ts</code>: Bump the npm-deps group across 1\ndirectory with 9 updates...</li>\n<li><a\nhref=\"https://github.com/DeterminateSystems/nix-installer-action/commit/500e7f9345c8f2d9f4e9f553c820978e6761c66b\"><code>500e7f9</code></a>\nUpdate <code>detsys-ts</code> for: <code>Bump fast-xml-parser from 5.3.3\nto 5.3.4\n([#134](https://github.com/determinatesystems/nix-installer-action/issues/134))</code>\n(`1...</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/determinatesystems/nix-installer-action/compare/c5a866b6ab867e88becbed4467b93592bce69f8a...ef8a148080ab6020fd15196c2084a2eea5ff2d25\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\nUpdates `actions/configure-pages` from 5 to 6\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/actions/configure-pages/releases\">actions/configure-pages's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v6.0.0</h2>\n<h1>Changelog</h1>\n<ul>\n<li>upgrade to node 24 <a\nhref=\"https://github.com/salmanmkc\"><code>@​salmanmkc</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/186\">#186</a>)</li>\n<li>Upgrade IA Publish <a\nhref=\"https://github.com/Jcambass\"><code>@​Jcambass</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/165\">#165</a>)</li>\n<li>Add workflow file for publishing releases to immutable action\npackage <a\nhref=\"https://github.com/Jcambass\"><code>@​Jcambass</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/163\">#163</a>)</li>\n<li>pin draft release version <a\nhref=\"https://github.com/YiMysty\"><code>@​YiMysty</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/162\">#162</a>)</li>\n<li>Bump espree from 9.6.1 to 10.1.0 <a\nhref=\"https://github.com/dependabot\"><code>@​dependabot</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/160\">#160</a>)</li>\n<li>Bump eslint-config-prettier from 8.8.0 to 9.1.0 <a\nhref=\"https://github.com/dependabot\"><code>@​dependabot</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/143\">#143</a>)</li>\n<li>Be more friendly to Dependabot <a\nhref=\"https://github.com/yoannchaudet\"><code>@​yoannchaudet</code></a>\n(<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/158\">#158</a>)</li>\n<li>Bump eslint-plugin-github from 4.10.2 to 5.0.1 <a\nhref=\"https://github.com/dependabot\"><code>@​dependabot</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/154\">#154</a>)</li>\n<li>Bump braces from 3.0.2 to 3.0.3 in the npm_and_yarn group <a\nhref=\"https://github.com/dependabot\"><code>@​dependabot</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/156\">#156</a>)</li>\n<li>Bump undici from 5.28.3 to 5.28.4 <a\nhref=\"https://github.com/dependabot\"><code>@​dependabot</code></a> (<a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/145\">#145</a>)</li>\n</ul>\n<p>See details of <a\nhref=\"https://github.com/actions/configure-pages/compare/v5.0.0...v5.0.1\">all\ncode changes</a> since previous release.</p>\n</blockquote>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/45bfe0192ca1faeb007ade9deae92b16b8254a0d\"><code>45bfe01</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/186\">#186</a>\nfrom salmanmkc/node24</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/d8770c2b3b71963902cec525cf516368b4411a78\"><code>d8770c2</code></a>\nUpdate Node version from 20 to 24 in action.yml</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/cb8a1a32801e6cdb7b111ce13761226bba88f67d\"><code>cb8a1a3</code></a>\nupgrade to node 24</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/d5606572c479bee637007364c6b4800ac4fc8573\"><code>d560657</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/165\">#165</a>\nfrom actions/Jcambass-patch-1</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/35e0ac4e4038e070ce9da26f41143bc3cf3c7e1d\"><code>35e0ac4</code></a>\nUpgrade IA Publish</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/1dfbcbff6519463927204dc279c2e0d307824ee2\"><code>1dfbcbf</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/163\">#163</a>\nfrom actions/Jcambass-patch-1</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/2f4f988792f75a5edcc39df0e1661f78999e0348\"><code>2f4f988</code></a>\nAdd workflow file for publishing releases to immutable action\npackage</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/0d7570ca8762e8c951911e8c9655d8973cc93174\"><code>0d7570c</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/162\">#162</a>\nfrom actions/pin-draft-release-verssion</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/3ea19669a5cd11c46d23d6578d088b81fe8527e5\"><code>3ea1966</code></a>\npin draft release version</li>\n<li><a\nhref=\"https://github.com/actions/configure-pages/commit/aabcbc432d6b06d1fd5e8bf3cf756880c35e014d\"><code>aabcbc4</code></a>\nMerge pull request <a\nhref=\"https://redirect.github.com/actions/configure-pages/issues/160\">#160</a>\nfrom actions/dependabot/npm_and_yarn/espree-10.1.0</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/actions/configure-pages/compare/v5...v6\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\nUpdates `benchmark-action/github-action-benchmark` from 1.21.0 to 1.22.0\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases\">benchmark-action/github-action-benchmark's\nreleases</a>.</em></p>\n<blockquote>\n<h2>v1.22.0</h2>\n<ul>\n<li><strong>chore</strong> bump node to 24 (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/339\">#339</a>)</li>\n</ul>\n<p><strong>Full Changelog</strong>: <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/compare/v1.21.0...v1.22.0\">https://github.com/benchmark-action/github-action-benchmark/compare/v1.21.0...v1.22.0</a></p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/blob/master/CHANGELOG.md\">benchmark-action/github-action-benchmark's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>Unreleased</h2>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.22.0\">v1.22.0</a>\n- 31 Mar 2026</h1>\n<ul>\n<li><strong>chore</strong> bump node to 24 (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/339\">#339</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.21.0\">v1.21.0</a>\n- 02 Mar 2026</h1>\n<ul>\n<li><strong>fix</strong> include package name for duplicate bench names\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/330\">#330</a>)</li>\n<li><strong>fix</strong> avoid duplicate package suffix in Go benchmarks\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/337\">#337</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.7\">v1.20.7</a>\n- 06 Sep 2025</h1>\n<ul>\n<li><strong>fix</strong> improve parsing for custom benchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/323\">#323</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.5\">v1.20.5</a>\n- 02 Sep 2025</h1>\n<ul>\n<li><strong>feat</strong> allow to parse generic cargo bench/criterion\nunits (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/280\">#280</a>)</li>\n<li><strong>fix</strong> add summary even when failure threshold is\nsurpassed (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/285\">#285</a>)</li>\n<li><strong>fix</strong> time units are not normalized (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/318\">#318</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.4\">v1.20.4</a>\n- 23 Oct 2024</h1>\n<ul>\n<li><strong>feat</strong> add typings and validation workflow (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/257\">#257</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.3\">v1.20.3</a>\n- 19 May 2024</h1>\n<ul>\n<li><strong>fix</strong> Catch2 v.3.5.0 changed output format (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/247\">#247</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.2\">v1.20.2</a>\n- 19 May 2024</h1>\n<ul>\n<li><strong>fix</strong> Support sub-nanosecond precision on Cargo\nbenchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/246\">#246</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.1\">v1.20.1</a>\n- 02 Apr 2024</h1>\n<ul>\n<li><strong>fix</strong> release script</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.20.0\">v1.20.0</a>\n- 02 Apr 2024</h1>\n<ul>\n<li><strong>fix</strong> Rust benchmarks not comparing to baseline (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/235\">#235</a>)</li>\n<li><strong>feat</strong> Comment on PR and auto update comment (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/223\">#223</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.19.3\">v1.19.3</a>\n- 02 Feb 2024</h1>\n<ul>\n<li><strong>fix</strong> ratio is NaN when previous value is 0. Now,\nprint 1 when both values are 0 and <code>+-∞</code> when divisor is 0\n(<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/222\">#222</a>)</li>\n<li><strong>fix</strong> action hangs in some cases for go fiber\nbenchmarks (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/225\">#225</a>)</li>\n</ul>\n<p><!-- raw HTML omitted --><!-- raw HTML omitted --></p>\n<h1><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/releases/tag/v1.19.2\">v1.19.2</a>\n- 26 Jan 2024</h1>\n<ul>\n<li><strong>fix</strong> markdown rendering for summary is broken (<a\nhref=\"https://redirect.github.com/benchmark-action/github-action-benchmark/issues/218\">#218</a>)</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/commit/a60cea5bc7b49e15c1f58f411161f99e0df48372\"><code>a60cea5</code></a>\nrelease v1.22.0</li>\n<li>See full diff in <a\nhref=\"https://github.com/benchmark-action/github-action-benchmark/compare/a7bc2366eda11037936ea57d811a43b3418d3073...a60cea5bc7b49e15c1f58f411161f99e0df48372\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore <dependency name> major version` will close this\ngroup update PR and stop Dependabot creating any more for the specific\ndependency's major version (unless you unignore this specific\ndependency's major version or upgrade to it yourself)\n- `@dependabot ignore <dependency name> minor version` will close this\ngroup update PR and stop Dependabot creating any more for the specific\ndependency's minor version (unless you unignore this specific\ndependency's minor version or upgrade to it yourself)\n- `@dependabot ignore <dependency name>` will close this group update PR\nand stop Dependabot creating any more for the specific dependency\n(unless you unignore this specific dependency or upgrade to it yourself)\n- `@dependabot unignore <dependency name>` will remove all of the ignore\nconditions of the specified dependency\n- `@dependabot unignore <dependency name> <ignore condition>` will\nremove the ignore condition of the specified dependency and ignore\nconditions\n\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-04-02T05:01:21Z",
          "url": "https://github.com/ava-labs/firewood/commit/25990da8a1407b6f91cba2a22180e73651dbe3ae"
        },
        "date": 1775115089558,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 167.3789781688242,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5974.465915255891,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.92770795084384,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5782.219995502993,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 78.63044968741912,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1915d09963c76b23397b92c7b38f122d8b511725",
          "message": "fix(proofs): yield divergent nodes from PathIterator for exclusion proofs (#1875)\n\n## Why this should be merged\n\nPreviously, PathIterator returned None whenever a node's partial path\ndiverged from the target key, at any depth. prove() compensated for\nroot-level divergence by manually constructing a ProofNode when the\niterator yielded nothing, but non-root divergent children were simply\nomitted from proofs.\n\nDiscovered during change proof implementation.\n\n## How this works\n\nNow PathIterator always yields divergent nodes regardless of depth. This\nis useful for exclusion proofs — a divergent node demonstrates that a\ndifferent key occupies the position where the proven key would be. This\nalso lets prove() collapse from 35 lines of special-case logic into a\nsimple iterator pipeline, removing a TODO about duplicated ProofNode\nconstruction, and simplifying prove() to be a simple pipeline.\n\nThe PathIterator::next() method is also restructured: the nested \\`match\ncomparison\\` + \\`match node\\` is replaced with early returns,\neliminating duplicated code between the divergent and leaf cases.\n\n## How this was tested\n\nNew unit test `path_iterate_yields_non_root_divergent_node` verifies\nthis behavior.\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-04-02T20:14:57Z",
          "url": "https://github.com/ava-labs/firewood/commit/1915d09963c76b23397b92c7b38f122d8b511725"
        },
        "date": 1775201517500,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 168.44964433365638,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5936.492201902496,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.47564980030867,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5745.976308027006,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 77.4175549959749,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1915d09963c76b23397b92c7b38f122d8b511725",
          "message": "fix(proofs): yield divergent nodes from PathIterator for exclusion proofs (#1875)\n\n## Why this should be merged\n\nPreviously, PathIterator returned None whenever a node's partial path\ndiverged from the target key, at any depth. prove() compensated for\nroot-level divergence by manually constructing a ProofNode when the\niterator yielded nothing, but non-root divergent children were simply\nomitted from proofs.\n\nDiscovered during change proof implementation.\n\n## How this works\n\nNow PathIterator always yields divergent nodes regardless of depth. This\nis useful for exclusion proofs — a divergent node demonstrates that a\ndifferent key occupies the position where the proven key would be. This\nalso lets prove() collapse from 35 lines of special-case logic into a\nsimple iterator pipeline, removing a TODO about duplicated ProofNode\nconstruction, and simplifying prove() to be a simple pipeline.\n\nThe PathIterator::next() method is also restructured: the nested \\`match\ncomparison\\` + \\`match node\\` is replaced with early returns,\neliminating duplicated code between the divergent and leaf cases.\n\n## How this was tested\n\nNew unit test `path_iterate_yields_non_root_divergent_node` verifies\nthis behavior.\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-04-02T20:14:57Z",
          "url": "https://github.com/ava-labs/firewood/commit/1915d09963c76b23397b92c7b38f122d8b511725"
        },
        "date": 1775461173163,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 173.77399251879865,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5754.601051085486,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.71623343604963,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5568.993476724316,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 74.60364021542223,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9e2fc0c78820eb131e621e2c273c68640a5ddef7",
          "message": "refactor(ffi): remove ArcCache from DatabaseHandle (#1891)\n\n## Why this should be merged\n\nArcCache was introduced to avoid redundant Db::view() calls during\ncommit by caching the proposal view before committing and clearing it\nafter. Due to subsequent API changes the cache has a 100% miss rate, so\nit provides no benefit while serializing all concurrent FFI view lookups\nbehind a Mutex held across the Db::view() factory call.\n\n## How this works\n\nReplace get_root() with a thin view() method that delegates directly to\nDb::view(), remove clear_cached_view() and the surrounding\ncommit_proposal() bookkeeping, and delete arc_cache.rs along with the\nCACHED_VIEW_MISS/HIT metrics.\n\n## How this was tested\n\nCI for integration tests\n\n## Breaking Changes\n\nn/a",
          "timestamp": "2026-04-07T03:09:15Z",
          "url": "https://github.com/ava-labs/firewood/commit/9e2fc0c78820eb131e621e2c273c68640a5ddef7"
        },
        "date": 1775547091643,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 169.13458653407523,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5912.451264357643,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 109.9352126853406,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5723.408754814351,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 76.55587550224865,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ron Kuris",
            "username": "rkuris",
            "email": "ron.kuris@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "faf306557759b7e322a7e0c96b0fea209eb082c2",
          "message": "chore(ci): Avoid running benchmarks on normal nextest (#1903)\n\n## Why this should be merged\n\nThe benchmarks run locally, take a long time, and the results are\nactually discarded. The only thing this actually checks is that the\nbenchmarks don't fail.\n\nWithout nextest, the benchmarks never run, so this seems like a useful\nimprovement.\n\n## How this works\n\nJust exclude benches from nextest\n\n## How this was tested\n\ncargo nextest is faster now\n\n## Breaking Changes\n\nNone",
          "timestamp": "2026-04-08T01:13:38Z",
          "url": "https://github.com/ava-labs/firewood/commit/faf306557759b7e322a7e0c96b0fea209eb082c2"
        },
        "date": 1775633773604,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 164.16784280983788,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6091.326918136701,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.49318092959055,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5889.822431551212,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 83.73697221306969,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1ffa782a948af617a4a9c252550ab0959a8fb57b",
          "message": "chore(deps): bump go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp from 1.22.0 to 1.43.0 in /ffi/tests/firewood (#1907)\n\nBumps\n[go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp](https://github.com/open-telemetry/opentelemetry-go)\nfrom 1.22.0 to 1.43.0.\n<details>\n<summary>Release notes</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/releases\">go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp's\nreleases</a>.</em></p>\n<blockquote>\n<h2>Release v1.23.0-rc.1</h2>\n<p>This is a release candidate for the v1.23.0 release. That release is\nexpected to include the <code>v1</code> release of the following\nmodules:</p>\n<ul>\n<li><code>go.opentelemetry.io/otel/bridge/opencensus</code></li>\n<li><code>go.opentelemetry.io/otel/bridge/opencensus/test</code></li>\n<li><code>go.opentelemetry.io/otel/example/opencensus</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetricgrpc</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp</code></li>\n\n<li><code>go.opentelemetry.io/otel/exporters/stdout/stdoutmetric</code></li>\n</ul>\n<p>See our <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/8f2bdf85ed99c6532b8c76688e7ffcf9e48c3e6d/VERSIONING.md\">versioning\npolicy</a> for more information about these stability guarantees.</p>\n</blockquote>\n</details>\n<details>\n<summary>Changelog</summary>\n<p><em>Sourced from <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/main/CHANGELOG.md\">go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp's\nchangelog</a>.</em></p>\n<blockquote>\n<h2>[1.43.0/0.65.0/0.19.0] 2026-04-02</h2>\n<h3>Added</h3>\n<ul>\n<li>Add <code>IsRandom</code> and <code>WithRandom</code> on\n<code>TraceFlags</code>, and <code>IsRandom</code> on\n<code>SpanContext</code> in <code>go.opentelemetry.io/otel/trace</code>\nfor <a\nhref=\"https://www.w3.org/TR/trace-context-2/#random-trace-id-flag\">W3C\nTrace Context Level 2 Random Trace ID Flag</a> support. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8012\">#8012</a>)</li>\n<li>Add service detection with <code>WithService</code> in\n<code>go.opentelemetry.io/otel/sdk/resource</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7642\">#7642</a>)</li>\n<li>Add <code>DefaultWithContext</code> and\n<code>EnvironmentWithContext</code> in\n<code>go.opentelemetry.io/otel/sdk/resource</code> to support plumbing\n<code>context.Context</code> through default and environment detectors.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8051\">#8051</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracegrpc</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetricgrpc</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploggrpc</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploghttp</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Support attributes with empty value (<code>attribute.EMPTY</code>)\nin\n<code>go.opentelemetry.io/otel/sdk/metric/metricdata/metricdatatest</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Add support for per-series start time tracking for cumulative\nmetrics in <code>go.opentelemetry.io/otel/sdk/metric</code>.\nSet <code>OTEL_GO_X_PER_SERIES_START_TIMESTAMPS=true</code> to enable.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8060\">#8060</a>)</li>\n<li>Add <code>WithCardinalityLimitSelector</code> for metric reader for\nconfiguring cardinality limits specific to the instrument kind. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7855\">#7855</a>)</li>\n</ul>\n<h3>Changed</h3>\n<ul>\n<li>Introduce the <code>EMPTY</code> Type in\n<code>go.opentelemetry.io/otel/attribute</code> to reflect that an empty\nvalue is now a valid value, with <code>INVALID</code> remaining as a\ndeprecated alias of <code>EMPTY</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n<li>Improve slice handling in\n<code>go.opentelemetry.io/otel/attribute</code> to optimize short slice\nvalues with fixed-size fast paths. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8039\">#8039</a>)</li>\n<li>Improve performance of span metric recording in\n<code>go.opentelemetry.io/otel/sdk/trace</code> by returning early if\nself-observability is not enabled. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8067\">#8067</a>)</li>\n<li>Improve formatting of metric data diffs in\n<code>go.opentelemetry.io/otel/sdk/metric/metricdata/metricdatatest</code>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8073\">#8073</a>)</li>\n</ul>\n<h3>Deprecated</h3>\n<ul>\n<li>Deprecate <code>INVALID</code> in\n<code>go.opentelemetry.io/otel/attribute</code>. Use <code>EMPTY</code>\ninstead. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8038\">#8038</a>)</li>\n</ul>\n<h3>Fixed</h3>\n<ul>\n<li>Return spec-compliant <code>TraceIdRatioBased</code> description.\nThis is a breaking behavioral change, but it is necessary to\nmake the implementation <a\nhref=\"https://opentelemetry.io/docs/specs/otel/trace/sdk/#traceidratiobased\">spec-compliant</a>.\n(<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8027\">#8027</a>)</li>\n<li>Fix a race condition in\n<code>go.opentelemetry.io/otel/sdk/metric</code> where the lastvalue\naggregation could collect the value 0 even when no zero-value\nmeasurements were recorded. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8056\">#8056</a>)</li>\n<li>Limit HTTP response body to 4 MiB in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp</code>\nto mitigate excessive memory usage caused by a misconfigured or\nmalicious server.\nResponses exceeding the limit are treated as non-retryable errors. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8108\">#8108</a>)</li>\n<li>Limit HTTP response body to 4 MiB in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp</code>\nto mitigate excessive memory usage caused by a misconfigured or\nmalicious server.\nResponses exceeding the limit are treated as non-retryable errors. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8108\">#8108</a>)</li>\n<li>Limit HTTP response body to 4 MiB in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploghttp</code>\nto mitigate excessive memory usage caused by a misconfigured or\nmalicious server.\nResponses exceeding the limit are treated as non-retryable errors. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8108\">#8108</a>)</li>\n<li><code>WithHostID</code> detector in\n<code>go.opentelemetry.io/otel/sdk/resource</code> to use full path for\n<code>kenv</code> command on BSD. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8113\">#8113</a>)</li>\n<li>Fix missing <code>request.GetBody</code> in\n<code>go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploghttp</code>\nto correctly handle HTTP2 GOAWAY frame. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8096\">#8096</a>)</li>\n</ul>\n<h2>[1.42.0/0.64.0/0.18.0/0.0.16] 2026-03-06</h2>\n<h3>Added</h3>\n<ul>\n<li>Add <code>go.opentelemetry.io/otel/semconv/v1.40.0</code> package.\nThe package contains semantic conventions from the <code>v1.40.0</code>\nversion of the OpenTelemetry Semantic Conventions.\nSee the <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/blob/main/semconv/v1.40.0/MIGRATION.md\">migration\ndocumentation</a> for information on how to upgrade from\n<code>go.opentelemetry.io/otel/semconv/v1.39.0</code>. (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/7985\">#7985</a>)</li>\n</ul>\n<!-- raw HTML omitted -->\n</blockquote>\n<p>... (truncated)</p>\n</details>\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/9276201a64b623606e3eaa0d61ae8ee6d62756c0\"><code>9276201</code></a>\nRelease v1.43.0 / v0.65.0 / v0.19.0 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8128\">#8128</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/61b8c9466c4e6b17e69b622279fe9b63fb15c89a\"><code>61b8c94</code></a>\nchore(deps): update module github.com/mattn/go-runewidth to v0.0.22 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8131\">#8131</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/97a086e82ffe01502f4c620e9c447efa229e2a23\"><code>97a086e</code></a>\nchore(deps): update github.com/golangci/dupl digest to c99c5cf (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8122\">#8122</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/5e363de517dba6db62736b2f5cdef0e0929b4cd0\"><code>5e363de</code></a>\nlimit response body size for OTLP HTTP exporters (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8108\">#8108</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/35214b60138eac8dec97a2d2b851d8c8471680c7\"><code>35214b6</code></a>\nUse an absolute path when calling bsd kenv (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8113\">#8113</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/290024ceaf695f9cdbf29a0c6731a317d92bc361\"><code>290024c</code></a>\nfix(deps): update module google.golang.org/grpc to v1.80.0 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8121\">#8121</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/e70658e098033d6bb5ec1b399de16bbb2642f6dc\"><code>e70658e</code></a>\nfix: support getBody in otelploghttp (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8096\">#8096</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/4afe468e3b4859c949a1c1e8d92684d43d86ef8a\"><code>4afe468</code></a>\nfix(deps): update googleapis to 9d38bb4 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8117\">#8117</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/b9ca729776309e3c08fe700c131797a3b4d10634\"><code>b9ca729</code></a>\nchore(deps): update module github.com/go-git/go-git/v5 to v5.17.2 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8115\">#8115</a>)</li>\n<li><a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/commit/69472ec56cb7674d55ca2e2bcb04dea73228ab79\"><code>69472ec</code></a>\nchore(deps): update fossas/fossa-action action to v1.9.0 (<a\nhref=\"https://redirect.github.com/open-telemetry/opentelemetry-go/issues/8118\">#8118</a>)</li>\n<li>Additional commits viewable in <a\nhref=\"https://github.com/open-telemetry/opentelemetry-go/compare/v1.22.0...v1.43.0\">compare\nview</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility\nscore](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp&package-manager=go_modules&previous-version=1.22.0&new-version=1.43.0)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't\nalter it yourself. You can also trigger a rebase manually by commenting\n`@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits\nthat have been made to it\n- `@dependabot show <dependency name> ignore conditions` will show all\nof the ignore conditions of the specified dependency\n- `@dependabot ignore this major version` will close this PR and stop\nDependabot creating any more for this major version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop\nDependabot creating any more for this minor version (unless you reopen\nthe PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop\nDependabot creating any more for this dependency (unless you reopen the\nPR or upgrade to it yourself)\nYou can disable automated security fix PRs for this repo from the\n[Security Alerts\npage](https://github.com/ava-labs/firewood/network/alerts).\n\n</details>\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-04-08T19:47:21Z",
          "url": "https://github.com/ava-labs/firewood/commit/1ffa782a948af617a4a9c252550ab0959a8fb57b"
        },
        "date": 1775720226863,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 164.52797525812608,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6077.993717670879,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 114.63867650372566,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5877.498255118017,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 82.68192902798911,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Joachim Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "db634eead1af8a9c7fd770fc64aa1a6d0f556cf2",
          "message": "feat(metrics): export prometheus metrics as structured data (#1868)\n\n## Why this should be merged\n\nThe Go FFI layer's `Gatherer` type collects Prometheus metrics by\ncalling into\nRust to render the text format, then parsing that text back into\n`dto.MetricFamily` structs. This text round-trip is wasteful and, more\nimportantly, loses fidelity for Native Histograms — the Prometheus text\nformat\ncannot represent them without downgrading to a classic histogram.\n\nThis PR eliminates the round-trip by adding a new `fwd_gather_rendered`\nFFI\nfunction that returns structured metric data directly.\n`Gatherer.Gather()` now\ncalls this structured path. The old text-based implementation is\npreserved as\n`TextGatherer` and marked deprecated.\n\nAs a demonstration of the structured path, `ffi_gather_duration_seconds`\nis\nregistered as a Native Histogram so that its full resolution is\npreserved\nend-to-end.\n\n## How this works\n\nThe upstream `metrics-exporter-prometheus` crate only exposed a text\nrenderer.\nA [fork](https://github.com/demosdemon/metrics.git) adds\n`PrometheusHandle::render_snapshot_and_descriptions()`, which returns a\nsnapshot of the registry as structured `render::MetricFamily` values\ninstead of\na text blob. This is patched in via `[patch.crates-io]` in the workspace\n`Cargo.toml`; https://github.com/metrics-rs/metrics/pull/686 is the\nupstream PR.\n\nOn the Rust side, `ffi/src/value/rendered_metrics.rs` defines\nC-compatible\nowned types (`OwnedMetricFamily`, `OwnedMetric`, `OwnedMetricValue`,\netc.) that\nmirror the Prometheus data model, including both classic and native\nhistograms.\n`From` impls convert from the `render::*` types into these owned types.\n`fwd_gather_rendered` calls `gather_rendered_metrics`, which snapshots\nthe\nregistry, converts the result, records its own wall-clock duration as a\nNative\nHistogram observation, and returns the owned slice to the caller.\n`fwd_free_rendered_metrics` drops the slice and reclaims the memory.\n\nOn the Go side, `GatherRenderedMetrics` calls `fwd_gather_rendered`,\nwalks the\nreturned C structs via `unsafe.Pointer` casts, and constructs\n`dto.MetricFamily` protobuf values. Because all conversion happens\nbefore\n`fwd_free_rendered_metrics` is called, the C memory remains valid\nthroughout.\n\n## How this was tested\n\n- `TestGatherRenderedMetrics` (new) exercises the full structured path:\nit\n  calls `GatherRenderedMetrics` several times, locates the\n`ffi_gather_duration_seconds` Native Histogram family, and asserts that\nthe\nsample count, sample sum, schema, zero threshold, and bucket spans are\nall\n  populated correctly.\n- Existing `TestMetrics` continues to pass, covering the deprecated text\npath.\n- `cargo nextest run --workspace --features ethhash,logger\n--all-targets`\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [x] firewood-ffi (C api) — adds `fwd_gather_rendered` and\n`fwd_free_rendered_metrics`; new C structs for structured metric data\n- [x] firewood-go (Go api) — `Gatherer.Gather()` now uses the structured\npath; `TextGatherer` added as deprecated alias for the old behaviour\n- [ ] fwdctl",
          "timestamp": "2026-04-09T20:38:43Z",
          "url": "https://github.com/ava-labs/firewood/commit/db634eead1af8a9c7fd770fc64aa1a6d0f556cf2"
        },
        "date": 1775807002331,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 162.1678208235139,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6166.451487858945,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 117.04943797871991,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5960.049691796236,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 85.92726913210389,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}