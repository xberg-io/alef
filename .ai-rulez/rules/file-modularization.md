---
type: Decision
title: File Modularization
x-ai-rulez:
  kind: rule
  id: file-modularization
  metadata:
    priority: high
---

Backend, codegen, e2e generator, and test source files must stay at or below 1,000 lines of
code, including tests. Files approaching 800 lines should be split before more behavior is
added. Split by concern, not by arbitrary line count.

Existing files over 1,000 lines are remediation targets and must not grow except in commits whose
purpose is splitting them. When touching an over-limit file, either split the touched concern
into a smaller module/test file or explicitly keep the change no-growth and preparatory.

115 files were already over the cap when `tests/file_size_ratchet.rs` landed and are grandfathered
in its baseline (`tests/file_size_baseline.txt`, `MAX_LINES = 1_000`) — the cap is not universally
met today. The ratchet is the enforcement: it fails on any new file over the cap and on any
baseline file that grows past its recorded ceiling, not on the grandfathered files simply existing.

Standard module structure for `src/backends/<lang>/` is documented in the `architecture` context
entry — split new backend code along that layout.

Functions exceeding 50 lines should be extracted into named helpers. Deeply nested conditional
blocks (>3 levels) should be extracted. When a file handles multiple distinct concepts, split it
at the concept boundary — not by line count alone. The 1,000-line cap applies to `src/**/*.rs`,
`src/**/*.jinja`, and `tests/**/*.rs`; generated snapshots are excluded.
