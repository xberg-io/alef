const assert = require("node:assert/strict");
const addon = require("./index.node");
const valid = { operation: "Analyze", policy: "ok" };
const invalid = { operation: "Count", policy: "bad" };
let checks = 0;
for (const name of ["direct", "borrowed", "optional", "optionalBorrowed"]) {
  assert.deepEqual(JSON.parse(addon[name](valid)), valid);
  checks++;
  assert.throws(
    () => addon[name](invalid),
    (error) => {
      assert.equal(error.code, "InvalidArg");
      assert.match(error.message, /invalid JSON.*invalid type/);
      assert.doesNotMatch(error.message, /\{err\}/);
      return true;
    },
  );
  checks++;
}
for (const name of ["vector", "optionalVector", "borrowedVector"]) {
  assert.deepEqual(JSON.parse(addon[name]([valid])), [valid]);
  checks++;
  assert.throws(
    () => addon[name]([valid, invalid]),
    (error) => error.code === "InvalidArg",
  );
  checks++;
}
assert.equal(JSON.parse(addon.optional()), null);
checks++;
assert.equal(JSON.parse(addon.optionalVector()), null);
checks++;
assert.equal(addon.outputOnly(), "ok");
checks++;
assert.deepEqual(addon.flattened({ kind: "Retry", retryCount: 2 }), { kind: "Retry", retryCount: 2 });
checks++;
assert.equal(checks, 18);
process.stdout.write(`passthrough enums: ${checks} checks passed\n`);
