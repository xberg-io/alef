---
priority: medium
---

When downstream consumer repos (tslp, kreuzcrawl, liter-llm, h2m, kreuzberg, kreuzberg-cloud) test a codegen change, use the local alef binary, not crates.io.

**Canonical install path:**

```bash
cargo install --path crates/alef-cli --force
```

Run this from the alef repo root. The resulting `~/.cargo/bin/alef` shadows any crates.io install on `$PATH`.

**Reinstall rules:**

- After every change to `alef-codegen`, any `alef-backend-*`, or `alef-cli` itself, re-run the install command above so the binary picks up the change.
- Do not reinstall from crates.io (`cargo install alef-cli`) until after a published release.
- Confirm the new binary is in use: `which alef` should point to `~/.cargo/bin/alef`, and `alef --version` should reflect your local changes.
