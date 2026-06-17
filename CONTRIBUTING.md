# Contributing to Alef

## Getting Started

1. Fork the repo and clone your fork.
2. Install the Rust toolchain: `rustup toolchain install stable`.
3. Run `task setup` to install development tools.
4. Run `task test` to verify your environment.

## Making Changes

- Follow the [conventional commits](https://www.conventionalcommits.org/) format.
- Run `prek run --all-files` before committing — hooks enforce formatting and linting.
- Keep commits atomic: one logical change per commit.
- Update `CHANGELOG.md` under `[Unreleased]` for every user-visible change.

## Pull Requests

- Open a PR against `main`.
- Describe **what** changed and **why** in the PR body.
- Link related issues with `Fixes #123` or `Refs #123`.
- CI must pass before merge.

## Reporting Issues

Use [GitHub Issues](https://github.com/kreuzberg-dev/alef/issues). Include:

- Alef version (`alef --version`)
- Minimal reproduction (input `alef.toml` + Rust source snippet)
- Expected vs actual output

## Community

Questions and discussion: [Discord](https://discord.gg/xt9WY3GnKR).
