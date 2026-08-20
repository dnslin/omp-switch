import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { createUiFixture, setScenario, writeSettings } from "./scripts/ui-fixture.mjs";

const binary = resolve(process.env.OMP_SWITCH_UI_BINARY ?? "src-tauri/target/release/omp-switch");
const outputDirectory = resolve(process.env.OMP_SWITCH_UI_OUTPUT ?? ".artifacts/issue-16/ui-acceptance");
const fixture = createUiFixture();
const fixtureRoot = fixture.root;
const target = fixture.target;
mkdirSync(outputDirectory, { recursive: true });

export const config = {
  runner: "local",
  specs: ["./scripts/ui-screenshot.e2e.mjs"],
  maxInstances: 1,
  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  services: [["@wdio/tauri-service", {
    appBinaryPath: binary,
    driverProvider: "embedded",
    embeddedPort: Number(process.env.TAURI_WEBDRIVER_PORT ?? 4445),
    startTimeout: 120000,
    commandTimeout: 60000,
    captureBackendLogs: true,
  }]],
  capabilities: [{
    browserName: "tauri",
    "tauri:options": { application: binary },
  }],
  waitforTimeout: 20000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 1,
  mochaOpts: { timeout: 120000 },
  onPrepare: () => {
    writeSettings();
    setScenario("overview");
  },
  before: async () => {
    const rawDpr = await browser.execute(() => window.devicePixelRatio);
    const dpr = typeof rawDpr === "number" ? rawDpr : rawDpr.value;
    let physicalWidth = Math.round(1536 * dpr);
    let physicalHeight = Math.round((process.platform === "darwin" ? 1088 : 1056) * dpr);
    let viewport = null;
    for (let attempt = 0; attempt < 6; attempt += 1) {
      await browser.setWindowSize(physicalWidth, physicalHeight);
      await browser.pause(250);
      const rawViewport = await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight, outerWidth: window.outerWidth, outerHeight: window.outerHeight }));
      viewport = rawViewport.value ?? rawViewport;
      if (viewport.width === 1536 && viewport.height === 1024) return;
      physicalWidth += Math.round((1536 - viewport.width) * dpr);
      physicalHeight += Math.round((1024 - viewport.height) * dpr);
    }
    throw new Error(`Unable to establish exact 1536x1024 CSS viewport; received inner ${viewport?.width ?? "unknown"}x${viewport?.height ?? "unknown"}, outer ${viewport?.outerWidth ?? "unknown"}x${viewport?.outerHeight ?? "unknown"} at dpr ${dpr}`);
  },
  onComplete: () => {
    fixture.cleanup();
  },
};

export { fixtureRoot, outputDirectory, target };
