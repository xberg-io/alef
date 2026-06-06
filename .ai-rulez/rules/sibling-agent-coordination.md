---
priority: medium
---

Assume other agents are active concurrently on this repo. Coordinate safely by following these rules.

- **Check freshness before pushing.** Use `git fetch` and inspect divergence before pushing. Do not run `git pull --rebase`, `git rebase`, or `git merge` after committing unless the user explicitly asks for it. If the branch has diverged, stop and ask how to reconcile it.
- **Never amend a commit that may already be in another agent's HEAD or pushed to the remote.** If you need to fix a commit message or contents after it's been pushed, create a new commit with the fix instead.
- **Never force-push to shared branches** (`main`, `master`, release branches). The only exception: retagging documented action tags in the owning Alef workflow/action repository when a critical action fix ships.
- **When you encounter unexpected files or branches,** investigate before deleting them. They may be another agent's work in progress.
