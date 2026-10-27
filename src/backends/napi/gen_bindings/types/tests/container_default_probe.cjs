const assert = require("node:assert/strict");
const { roundTrip, requiredRecord } = require("./index.node");

const defaults = { maxDepth: 7, browser: { mode: "auto", timeout: 1500 } };
assert.deepEqual(roundTrip({}), defaults);
assert.deepEqual(roundTrip({ maxDepth: 1 }), { ...defaults, maxDepth: 1 });
assert.deepEqual(roundTrip({ browser: { mode: "always" } }), {
  maxDepth: 7,
  browser: { mode: "always", timeout: 1500 },
});
assert.deepEqual(roundTrip({ browser: { timeout: 2500 } }), {
  maxDepth: 7,
  browser: { mode: "auto", timeout: 2500 },
});
assert.deepEqual(requiredRecord({ content: "kept", row: 3 }), { content: "kept", row: 3 });
assert.throws(() => requiredRecord({}), /content/);
process.stdout.write("6 native sparse/default/required controls passed\n");
