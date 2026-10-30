const assert = require("node:assert/strict");
const addon = require("./index.node");
const capsule = addon.getHandle();
assert.deepEqual(Object.keys(capsule), ["__parser"]);
assert.equal(addon.matchesTag(capsule.__parser, true), true);
assert.equal(addon.matchesTag(capsule.__parser, false), false);
assert.equal(addon.matchesTag({}, true), false);
process.stdout.write("capsule native: 3 tag checks passed\n");
