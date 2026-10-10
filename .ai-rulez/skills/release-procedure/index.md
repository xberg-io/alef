# Concepts

* [Release Procedure](SKILL.md) - Cut, tag, and publish an alef release end-to-end. Use this skill any time the user asks for a release, a version bump, a hotfix tag, or a CHANGELOG roll-up in this repo. Covers the full pipeline: changelog, version sync via Taskfile, Cargo.toml verification, poly lint pass, atomic commit (no AI signatures, no --no-verify when avoidable), git tag, and `gh release create` (not just a tag).
