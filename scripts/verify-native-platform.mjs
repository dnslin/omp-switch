import { spawnSync } from "node:child_process";

const [expectedPlatform, expectedArchitecture, expectedRustTarget] = process.argv.slice(2);
if (!expectedPlatform || !expectedArchitecture || !expectedRustTarget) {
  console.error("Usage: node scripts/verify-native-platform.mjs <platform> <architecture> <rust-target>");
  process.exit(2);
}

const rustVersion = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
if (rustVersion.status !== 0) {
  console.error(rustVersion.stderr || "rustc -vV failed");
  process.exit(rustVersion.status ?? 1);
}
const host = rustVersion.stdout.match(/^host:\s*(\S+)$/m)?.[1];
const actual = { platform: process.platform, architecture: process.arch, rustTarget: host ?? null };
if (actual.platform !== expectedPlatform || actual.architecture !== expectedArchitecture || actual.rustTarget !== expectedRustTarget) {
  console.error(JSON.stringify({ expected: { platform: expectedPlatform, architecture: expectedArchitecture, rustTarget: expectedRustTarget }, actual }, null, 2));
  process.exit(1);
}
console.log(JSON.stringify({ ...actual, native: true }));
