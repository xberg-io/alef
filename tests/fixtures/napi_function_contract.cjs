const assert = require("node:assert/strict");
const addon = require("./index.node");

async function main() {
  let checks = 0;
  const value = { nested: [null, true, 4, { text: "hello" }] };
  for (const name of ["jsonOwned", "jsonBorrowed"]) {
    for (const input of [value, "text", 5, true, null]) {
      assert.deepEqual(addon[name](input), input);
      checks++;
    }
  }
  assert.deepEqual(addon.echoOwner({ child: { value: "owned" } }), { child: { value: "owned" } });
  checks++;
  assert.deepEqual(addon.jsonWithPacket({}, value), value);
  checks++;
  for (const input of [{}, { values: [], fallbacks: [] }, { values: ["x"], fallbacks: ["y"] }]) {
    const output = addon.echoPacket(input);
    assert.deepEqual(output.values, input.values);
    assert.deepEqual(output.fallbacks, input.fallbacks);
    checks++;
  }
  for (const field of ["values", "fallbacks"]) {
    assert.throws(() => addon.echoPacket({ [field]: null }));
    checks++;
  }
  assert.equal(await addon.consumeUnit("ok"), undefined);
  assert.equal(await addon.consumeFallible("ok"), undefined);
  await assert.rejects(addon.consumeFallible("fail"), /core failure/);
  checks++;
  assert.equal(await addon.consumePromoted(undefined, "ok"), undefined);
  await assert.rejects(addon.consumePromoted(undefined, undefined), /missing required parameter/);
  assert.equal(addon.consumeSync("ok"), undefined);
  checks++;
  const rows = [{ nil: null, object: { nested: true }, number: 23, array: [null, 2, { text: "kept" }] }];
  const packets = [
    { flags: [], rows: [] },
    { flags: [true, false], rows },
    { flags: [false], rows, maybeRows: [] },
    { flags: [true, false, true], rows, maybeRows: rows },
  ];
  assert.equal(packets.length, 4);
  for (const input of packets) {
    const output = addon.echoMapPacket(input);
    assert.deepEqual(output.flags, input.flags);
    assert.deepEqual(output.rows, input.rows);
    assert.deepEqual(output.maybeRows, input.maybeRows);
    checks++;
  }
  assert.equal(checks, 23);
  process.stdout.write(`function contracts: ${checks} checks passed\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 1;
});
