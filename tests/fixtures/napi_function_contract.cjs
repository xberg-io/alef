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
  assert.equal(checks, 19);
  process.stdout.write(`function contracts: ${checks} checks passed\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 1;
});
