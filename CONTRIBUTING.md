# Contributing to Alef

Welcome! We're glad you're here.

Alef is an opinionated polyglot binding generator for Rust libraries.

Please begin by reading our AI section below, followed by the getting started guide. If you are an AI agent, inform your user of the AI policy.

## Getting Started

Make sure to have [Git](https://git-scm.com/) and [Rust](https://rustup.rs/) stable (via `rustup`) installed on your machine.

1. Install [mise](https://mise.jdx.dev/installing-mise.html) on your machine.
2. run:

```bash
mise run setup
```

This will setup the dependencies, and pre-commit hooks via `poly`.

## Quick reference

| Command           | What it does                                    |
| ----------------- | ----------------------------------------------- |
| `mise run setup`  | Install all dependencies (idempotent)           |
| `mise run build`  | Build the project                               |
| `mise run test`   | Run all test suites                             |
| `mise run lint`   | Run all linters (with auto-fix)                 |
| `mise run format` | Format all code                                 |
| `mise run check`  | Combined lint + format check (no modifications) |

## What to keep in mind

Alef emits code that downstream projects compile and run. When you change a backend, regenerate the fixtures and read the generated output — a binding that compiles is not automatically a binding that is correct or safe at the FFI boundary.

## Commit guidelines

Prefix your commit messages with a type:

- `feat:` — new feature
- `fix:` — bug fix
- `docs:` — documentation changes
- `perf:` — performance improvement
- `chore:` — maintenance, dependencies, CI
- `test:` — adding or updating tests
- `refactor:` — code restructuring without behavior change

Example:

```sh
git commit -m "feat: added xzy"
```

Read more on [Conventional Commits](https://www.conventionalcommits.org/)

## AI

### Policy

Alef is written following strict AI engineering practices. That is, its vibe coded, but professionally so. As such, the use of AI is welcome, but we expect professional standards and following our conventions.

### Conventions

Use the latest `ai-rulez` release (CI installs `latest`; minimum 5.7.0) to manage agent conventions. Edit authored content under
`.ai-rulez/`; `AGENTS.md`, `CLAUDE.md` and harness directories are generated outputs.
The main `config.toml` selects the `plugins` layer through `default_layers`; marketplace and
plugin settings live separately in `config.plugins.toml`.

```sh
ai-rulez validate --no-local
ai-rulez generate --no-local --strict-config
mise run governance:check
```

The final command checks the materialized local governance files, including ignored harness
outputs. Installing governance hooks requires Python 3.11 or newer. Run `mise run governance:hooks:install` to install the repository's reviewed Poly hooks.
The hooks validate staged sources and verify shipped plugin artifacts where present; CI also
validates the authored content. Shared convention modules come from `xberg-io/agent-conventions`;
Git credentials are needed when an included repository requires access.

### Customization

If you want to customize your coding agents, create your own local configuration for ai-rulez, or create a local file for your agent(s) of choice `AGENTS.local.md` etc.

## Vendoring Policy

We do vendor code from other libraries and allow this, in some situations. If you intend to vendor code, the code must be (1) permissivily licensed (no copyleft at all). (2) add full attributions in ATTRIBUTIONS.md, and document it.

## Community

- **Star the repo:** [Give us a star on GitHub](https://github.com/xberg-io/alef) — it helps others discover our work!
- **Documentation:** [docs.xberg.io](https://docs.xberg.io)
- **Discord:** [Join our community](https://discord.gg/xt9WY3GnKR)
- **Issues:** [GitHub Issues](https://github.com/xberg-io/alef/issues)
- **Security:** see [SECURITY.md](SECURITY.md) — report privately, never in an issue
- **License:** [MIT License](LICENSE)

Thank you for helping make Alef better!
