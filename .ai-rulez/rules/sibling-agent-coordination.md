---
type: Decision
title: Sibling Agent Coordination
x-ai-rulez:
  kind: rule
  id: sibling-agent-coordination
  metadata:
    priority: medium
---

Assume other agents are active concurrently on this repo. Coordinate safely by following these rules.

- **Check freshness before pushing.** Use `git fetch` and inspect divergence before pushing. Do not run `git pull --rebase`, `git rebase`, or `git merge` after committing unless the user explicitly asks for it. If the branch has diverged, stop and ask how to reconcile it.
- **Never amend a commit that may already be in another agent's HEAD or pushed to the remote.** If you need to fix a commit message or contents after it's been pushed, create a new commit with the fix instead.
- **Never force-push to shared branches** (`main`, `master`, release branches). The only exception: retagging documented action tags in the owning Alef workflow/action repository when a critical action fix ships.
- **When you encounter unexpected files or branches,** investigate before deleting them. They may be another agent's work in progress.
- **Worktree isolation forks the session's own repo, not necessarily the repo an agent is dispatched to work in.** For an agent targeting a different repo, isolation buys nothing — enforce one writer per repo by scheduling, and say so in the brief.
- **Re-run a subagent's tests yourself and rebuild first** before trusting its diagnosis — a stale build can make a passing test look like a failure, and "I ran the tool" is not the same claim as "I fed it the real artifact."
- **Verify a factual claim before it goes into a brief**, and add "if this diagnosis is wrong, say so and fix the real cause" — a detailed brief propagates the coordinator's own errors at full confidence to a compliant agent.
