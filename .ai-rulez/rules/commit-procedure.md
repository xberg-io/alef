---
priority: critical
---

Commit procedure is lightweight by default.

- Do not run `cargo check`, `cargo test`, or the full test suite as mandatory commit steps.
- Run `poly lint .` (and `poly fmt --fix .` to apply formatting) before committing. Re-stage files if fixes rewrite them.
- Run targeted verification when it is relevant to the change, requested by the user, or required by a release procedure.
- Do not run `git pull --rebase`, `git rebase`, or `git merge` after committing unless the user explicitly asks for it.
- Before pushing, check remote freshness with `git fetch` and inspect divergence. If the branch has diverged, ask the user how to reconcile it.
