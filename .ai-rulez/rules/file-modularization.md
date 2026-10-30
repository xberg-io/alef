---
type: Decision
title: File Modularization
paths:
  - "src/**/*.rs"
  - "src/**/*.jinja"
  - "tests/**/*.rs"
x-ai-rulez:
  kind: rule
  id: file-modularization
  metadata:
    priority: high
---

- Cap: 1,000 lines per file, tests included, for `src/**/*.rs`, `src/**/*.jinja`, `tests/**/*.rs`; generated snapshots are excluded. Split files nearing 800 lines before adding behavior.
- Split by concept boundary, not line count; backend code follows the `backend-modules` layout.
- `tests/file_size_ratchet.rs` enforces it against `tests/file_size_baseline.txt`: it fails on a new over-cap file or a baseline file growing past its ceiling.
- Over-cap baseline files do not grow except in commits that split them; when touching one, split the touched concern or keep the change no-growth.
- After splitting, `UPDATE_FILE_SIZE_BASELINE=1 cargo test --test file_size_ratchet -- --ignored` rewrites the baseline; `cargo test --test file_size_ratchet -- --ignored` runs the ratchet, whose tests carry `#[ignore]` while it is disabled (alef#482).
