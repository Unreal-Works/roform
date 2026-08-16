#!/usr/bin/env node

const { spawnSync } = require("node:child_process");

const platformPackage = `@unrealworks/roform-${process.platform}-${process.arch}`;
const binaryName = process.platform === "win32" ? "roform.exe" : "roform";

let binaryPath;
try {
  binaryPath = require.resolve(`${platformPackage}/bin/${binaryName}`);
} catch {
  console.error(
    `roform does not provide a binary for ${process.platform}-${process.arch}. ` +
      "Supported platforms are Windows, macOS, and Linux on x64 or arm64.",
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
});
if (result.error) {
  console.error(`failed to start roform: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
