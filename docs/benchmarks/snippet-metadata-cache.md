# Full-corpus snippet metadata benchmark

Measured on 2026-11-02 for [issue #753](https://github.com/xberg-io/alef/issues/753).

## Controlled comparison

Both binaries were built locally with Rust 1.99.1, release optimization level 3, LTO disabled,
debug information disabled, and incremental compilation disabled. The baseline is
`b9df875ff958575c826f7d3af83bfc820fcf1ba5`. The comparison applies only these existing performance commits
to that baseline, in order:

- [`eb6fa6cd4`](https://github.com/xberg-io/alef/commit/eb6fa6cd4db7be0de2c9fe1503cf97afb01c110d): batch enum payload names.
- [`586179ed9`](https://github.com/xberg-io/alef/commit/586179ed97726827be1e41be1790391506da1f87): avoid duplicate anchoring.
- [`d65161b4c`](https://github.com/xberg-io/alef/commit/d65161b4cc3a31cf5275efcf999bf6ed9b5be67b): share immutable session facts.

Both source trees were clean. Their manifests, schema, templates, version constants and lockfile were identical;
the lockfile SHA256 was `dd30ceca9d3ffb3077e9c4706b251db3c454010895115e69e1d03acbd891c376`.
The comparison tree hash was `7af919fbb37c65ff1b7f369eb2032442f58525d7`.

The consumer was the public HTML-to-Markdown `v3.19.1` commit
`f9aa4df48959473678a0c94f29d82939531b1f0d`, checked out independently for each run.
Each binary ran the exact command `alef verify --no-color`, without report-only mode or language filtering.
Each process began without a consumer `.alef` cache. A complete tracked-file digest pass primed filesystem reads;
no system-wide filesystem-cache flush was attempted. Installation, builds and downloads were outside the timings.

Host: Apple M4 Pro, 14 cores, 48 GiB RAM, macOS 27.0.1. Runs were sequential; no Cargo or rustc process was present
in the process snapshots. The one-minute load averages before the runs were 2.85 and 3.09.

| Binary | Exit | Wall seconds | User CPU seconds | System CPU seconds | Peak RSS MiB |
|---|---:|---:|---:|---:|---:|
| Baseline | 0 | 204.162 | 303.958 | 5.910 | 215.922 |
| Baseline plus the three performance commits | 0 | 74.494 | 113.612 | 5.071 | 216.094 |

This single pair used 63.51% less wall time and 61.70% less total CPU time for the same verification outcome.
These measurements describe this host and corpus; they do not estimate variance or other workloads.
Wall time came from a monotonic clock; CPU and peak RSS came from `wait4` resource usage.

## Work and output checks

Both runs checked 43 consistent version manifests and reported the same managed surface: 6,344 paths,
6,262 content-verified, 81 present without content verification, and one absent. Each loaded the same 358 fixtures
and produced three reports of 96 curated and 5,620 generated snippets: 16,860 generated cells in total.
Fresh-render comparisons covered 6,225 paths: 132 through a real formatter and 6,093 directly.
Another 29 paths used external formatter overrides whose byte output the verifier does not predict.

The one absent path was `packages/go/.lib/.gitkeep`, an untracked scaffold placeholder absent from the public tag.
The verifier reports it as coverage and does not fail on this headerless scaffold output.
The original issue's populated working checkout counted that path among its 82 present-only paths.
All 8,222 tracked corpus files retained the same digest before and after both runs:
`4e5e37ba596273539a16d9d881916ddeb1d2de93724278334d29248d3f5a9574`.

An untimed run with `RUST_LOG='info,[snippet_presentation]=trace'` counted the real presentation constructors:
fields, enum and collection facts each ran three times; result facts ran six times for the two optionality rules;
C# and Dart variant facts each ran three times. Those counts cover all three reports, rather than one construction
per snippet cell. Constructors outside the presentation session remain outside this cache's scope.

A separate negative control appended text to exactly one generated Python snippet. Verification exited 1 and
named that file under both stale-content and fresh-render drift findings. Restoring its exact original bytes
returned the corpus to its clean state; the controlled comparison subsequently passed on identical tracked bytes.

## Qualification boundary

An initial comparison of official 0.115.7 and 0.116.0 binaries took 198.676 and 77.359 wall seconds respectively.
The latter correctly rejected unrelated generator drift in the historical corpus and had a different managed
surface. Those measurements are excluded from the performance result above. The controlled builds retain the
original generation contract so the measured change is limited to the three metadata-performance commits.
