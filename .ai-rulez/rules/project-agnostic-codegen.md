---
priority: high
---

Alef must remain project-agnostic. Do not hard-code downstream project identities, repository names, package names, module names, paths, or product-specific branches in generator, extraction, scaffold, e2e, publish, or CLI behavior.

Downstream-specific differences must be modeled through generic configuration such as `alef.toml`, IR metadata, backend language config, feature flags, or reusable capability flags. If behavior is needed for one consumer, name and implement the underlying generic condition instead of matching that consumer's project name.

Regression tests may describe the generic behavior being protected, but fixtures and expected output should use neutral sample names rather than real downstream project names.
