import { access, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";

const argumentsList = process.argv.slice(2);
const binaryArgument = argumentsList[0];
const reportFlagIndex = argumentsList.indexOf("--report");
const reportPath = reportFlagIndex >= 0 ? argumentsList[reportFlagIndex + 1] : process.env.OMP_SWITCH_SMOKE_REPORT;

if (!binaryArgument || (reportFlagIndex >= 0 && !reportPath)) {
  console.error("Usage: node scripts/platform-smoke.mjs <release-binary> [--report <path>]");
  process.exit(2);
}

const binaryPath = resolve(binaryArgument);
await access(binaryPath);

const waitMs = Number(process.env.OMP_SWITCH_SMOKE_WAIT_MS ?? 5000);
const child = spawn(binaryPath, [], {
  cwd: process.cwd(),
  stdio: "ignore",
  windowsHide: true,
});

let exitResult = null;
const exited = new Promise((resolveExit) => {
  child.once("exit", (code, signal) => {
    exitResult = { code, signal };
    resolveExit(exitResult);
  });
});

await Promise.race([exited, delay(waitMs)]);
if (exitResult) {
  console.error(`Packaged application exited before the ${waitMs}ms launch window: ${JSON.stringify(exitResult)}`);
  process.exit(1);
}

const report = {
  platform: process.platform,
  architecture: process.arch,
  binaryPath,
  releaseAssetPath: process.env.OMP_SWITCH_RELEASE_ASSET ? resolve(process.env.OMP_SWITCH_RELEASE_ASSET) : null,
  releaseAssetName: process.env.OMP_SWITCH_RELEASE_ASSET ? basename(process.env.OMP_SWITCH_RELEASE_ASSET) : null,
  launchWindowMs: waitMs,
  launched: true,
};
if (reportPath) {
  await writeFile(resolve(reportPath), `${JSON.stringify(report, null, 2)}\n`, "utf8");
}
console.log(JSON.stringify(report));

if (process.platform === "win32") {
  const result = spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore", timeout: 5000 });
  if (result.status !== 0 || result.error) child.kill();
} else {
  child.kill("SIGTERM");
}
await Promise.race([exited, delay(2000)]);
if (!exitResult) child.kill("SIGKILL");
