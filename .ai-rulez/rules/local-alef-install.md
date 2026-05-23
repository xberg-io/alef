---
priority: medium
---

When downstream consumer repos (tslp, kreuzcrawl, liter-llm, h2m, kreuzberg, kreuzberg-cloud) test a codegen change, use the local alef binary, not crates.io.

**Canonical install path:**

```bash
cargo install --path . --force
```

Run this from the alef repo root. Alef is a single root-flat crate, so `--path .` installs the `alef` binary from the workspace root. The resulting `~/.cargo/bin/alef` shadows any crates.io install on `$PATH`.

**Reinstall rules:**

- After every change under `src/` (codegen, any backend, or CLI), re-run the install command above so the binary picks up the change.
- Do not reinstall from crates.io (`cargo install alef`) until after a published release.
- Confirm the new binary is in use: `which alef` should point to `~/.cargo/bin/alef`, and `alef --version` should reflect your local changes.
