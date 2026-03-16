window.BENCHMARK_DATA = {
  "lastUpdate": 1773647391330,
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
      }
    ]
  }
}