---
priority: high
---

Alef must remain project-agnostic. Do not hard-code consumer project identities, repository names, package names, module names, paths, or product-specific branches in generator, extraction, scaffold, e2e, publish, or CLI behavior.

Consumer-specific differences must be modeled through generic configuration such as `alef.toml`, IR metadata, backend language config, feature flags, or reusable capability flags. If behavior is needed for one consumer, name and implement the underlying generic condition instead of matching that consumer's project name.

Regression tests may describe the generic behavior being protected, but fixtures and expected output should use neutral sample names rather than real consumer project names.

TODO(alef-generic-cleanup): remove remaining domain-shaped examples and scaffold/e2e defaults after they are fixture/config-driven.

Generated snapshots, docs, and guidance must also use neutral fixture names. Do not preserve consumer domain types such as product-specific config, result, visitor, handle, or backend names in expected output; rename them to generic fixture concepts before accepting snapshots.
