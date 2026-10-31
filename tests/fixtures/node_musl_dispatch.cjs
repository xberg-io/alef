const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const vm = require("node:vm");

const seed = readFileSync(process.argv[2], "utf8");
assert.ok(seed.includes("const isMusl ="), "the actual emitted loader must be nonempty");
assert.ok(seed.includes('"./musl-probe-node.linux-arm64-musl.node"'), "the ARM64 musl dispatch row must exist");
const armLoader = "/lib/ld-musl-aarch64.so.1";
const x64Loader = "/lib/ld-musl-x86_64.so.1";
const runtimeReport = { header: { glibcVersionRuntime: "2.17" } };
const cases = [
  { name: "arm64-lib", files: [armLoader], target: "linux-arm64-musl" },
  { name: "arm64-lib64", files: ["/lib64/ld-musl-aarch64.so.1"], target: "linux-arm64-musl" },
  { name: "x64-lib", arch: "x64", files: [x64Loader], target: "linux-x64-musl" },
  { name: "x64-lib64", arch: "x64", files: ["/lib64/ld-musl-x86_64.so.1"], target: "linux-x64-musl" },
  {
    name: "glibc-runtime-authoritative",
    arch: "x64",
    report: runtimeReport,
    files: ["/lib64/ld-musl-x86_64.so.1"],
    target: "linux-x64-gnu",
    noFilesystem: true,
  },
  {
    name: "glibc-legacy-authoritative",
    arch: "x64",
    report: { header: { glibcVersion: "2.17" } },
    files: ["/lib64/ld-musl-x86_64.so.1"],
    target: "linux-x64-gnu",
    noFilesystem: true,
  },
  {
    name: "musl-shared-object",
    report: { sharedObjects: [armLoader] },
    target: "linux-arm64-musl",
    noFilesystem: true,
  },
  { name: "report-absent-ldd", reportMode: "absent", ldd: "musl libc", target: "linux-arm64-musl" },
  { name: "report-throws-fallback", reportMode: "throws", files: [armLoader], target: "linux-arm64-musl" },
  { name: "null-report-fallback", reportMode: "null", files: [armLoader], target: "linux-arm64-musl" },
  { name: "gnu-ldd", reportMode: "absent", ldd: "GNU libc", target: "linux-arm64-gnu" },
  { name: "unknown-libc-default", reportMode: "absent", target: "linux-arm64-gnu" },
  { name: "darwin-native", platform: "darwin", target: "darwin-arm64", noFilesystem: true },
  { name: "windows-native", platform: "win32", arch: "x64", target: "win32-x64-msvc", noFilesystem: true },
  { name: "gnu-optional-package", report: runtimeReport, optional: true, target: "linux-arm64-gnu" },
  { name: "musl-optional-package", report: { sharedObjects: [armLoader] }, optional: true, target: "linux-arm64-musl" },
];
assert.equal(cases.length, 16);

function runCase(testCase) {
  let filesystemCalls = 0;
  const missing = () => Object.assign(new Error("missing fixture path"), { code: "ENOENT" });
  const filesystem = {
    statSync(path) {
      filesystemCalls++;
      if (!(testCase.files || []).includes(path)) throw missing();
      return {};
    },
    readFileSync(path) {
      filesystemCalls++;
      assert.equal(path, "/usr/bin/ldd");
      if (testCase.ldd === undefined) throw missing();
      return testCase.ldd;
    },
  };
  const report =
    testCase.reportMode === "absent"
      ? undefined
      : {
          getReport() {
            if (testCase.reportMode === "throws") throw new Error("report unavailable");
            return testCase.reportMode === "null" ? null : testCase.report || { header: {} };
          },
        };
  const module = { exports: {} };
  const context = {
    module,
    exports: module.exports,
    process: { platform: testCase.platform || "linux", arch: testCase.arch || "arm64", report },
    require(name) {
      if (name === "fs") return filesystem;
      if (name === "./binding.cjs") return (binding) => ({ ...binding, shimApplied: true });
      const local = name.match(/^\.\/musl-probe-node\.(.+)\.node$/);
      if (local && !testCase.optional) return { target: local[1], source: "local" };
      if (local) throw missing();
      if (name.startsWith("musl-probe-")) return { target: name.slice("musl-probe-".length), source: "package" };
      throw new Error(`unexpected fixture require: ${name}`);
    },
  };
  vm.runInNewContext(seed, context, { timeout: 1000 });
  assert.equal(module.exports.target, testCase.target, testCase.name);
  assert.equal(module.exports.source, testCase.optional ? "package" : "local", testCase.name);
  assert.equal(module.exports.shimApplied, true, "the error-normalization shim must remain attached");
  if (testCase.noFilesystem) assert.equal(filesystemCalls, 0, testCase.name);
}

const failures = [];
for (const testCase of cases) {
  try {
    runCase(testCase);
  } catch (error) {
    failures.push({ name: testCase.name, error: error.message });
  }
}
process.stdout.write(JSON.stringify({ total: cases.length, passed: cases.length - failures.length, failures }) + "\n");
assert.equal(failures.length, 0, "every actual emitted-seed dispatch case must pass");
