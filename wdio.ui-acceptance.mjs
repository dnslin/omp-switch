import { mkdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { createUiFixture, setScenario, writeSettings } from "./scripts/ui-fixture.mjs";

const binary = resolve(process.env.OMP_SWITCH_UI_BINARY ?? "src-tauri/target/release/omp-switch");
const outputDirectory = resolve(process.env.OMP_SWITCH_UI_OUTPUT ?? ".artifacts/issue-16/ui-acceptance");
const evidenceHeight = Number(process.env.OMP_SWITCH_UI_HEIGHT ?? "960");
if (![960, 1024].includes(evidenceHeight)) throw new Error(`OMP_SWITCH_UI_HEIGHT must be 960 or 1024; received ${evidenceHeight}`);
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
    const targetContentWidth = Math.round(1536 * dpr);
    const targetContentHeight = Math.round(evidenceHeight * dpr);
    let physicalWidth = targetContentWidth;
    let physicalHeight = Math.round((evidenceHeight + 64) * dpr);
    let outerRect = null;
    let contentViewport = null;
    let screenshotSize = null;
    const attempts = [];
    for (let attempt = 0; attempt < 8; attempt += 1) {
      await browser.setWindowSize(physicalWidth, physicalHeight);
      await browser.pause(250);
      const rawRect = await browser.getWindowSize();
      outerRect = rawRect.value ?? rawRect;
      const rawViewport = await browser.execute(() => ({
        width: window.innerWidth,
        height: window.innerHeight,
        visualWidth: window.visualViewport?.width ?? null,
        visualHeight: window.visualViewport?.height ?? null,
      }));
      contentViewport = rawViewport.value ?? rawViewport;
      const rawScreenshot = await browser.takeScreenshot();
      const screenshot = typeof rawScreenshot === "string" ? rawScreenshot : rawScreenshot.value;
      const rawSize = await browser.executeAsync((source, done) => {
        const image = new Image();
        image.onload = () => done({ width: image.naturalWidth, height: image.naturalHeight });
        image.onerror = () => done({ error: "screenshot-decode-failed" });
        image.src = `data:image/png;base64,${source}`;
      }, screenshot);
      screenshotSize = rawSize.value ?? rawSize;
      attempts.push({ attempt, requested: { width: physicalWidth, height: physicalHeight }, outer: outerRect, content: contentViewport, snapshot: screenshotSize });
      if (
        contentViewport.width === 1536 &&
        contentViewport.height === evidenceHeight &&
        screenshotSize.width >= targetContentWidth &&
        screenshotSize.height >= targetContentHeight
      ) {
        writeFileSync(join(outputDirectory, "raw-snapshot.png"), Buffer.from(screenshot, "base64"));
        writeFileSync(join(outputDirectory, "window-geometry.json"), `${JSON.stringify({
          devicePixelRatio: dpr,
          requestedNativeWindowBaseline: { width: 1536, height: 1024 },
          requestedWebviewViewport: { width: 1536, height: evidenceHeight },
          actualOuterWindow: { physicalWidth: outerRect.width, physicalHeight: outerRect.height },
          webviewContent: { width: contentViewport.width, height: contentViewport.height },
          rawSnapshot: { width: screenshotSize.width, height: screenshotSize.height },
          normalizedSnapshot: { width: 1536, height: evidenceHeight },
        }, null, 2)}\n`);
        return;
      }
      if (!screenshotSize.width || !screenshotSize.height) break;
      physicalWidth += Math.round((1536 - contentViewport.width) * dpr);
      physicalHeight += Math.round((evidenceHeight - contentViewport.height) * dpr);
      if (screenshotSize.width < targetContentWidth) physicalWidth += targetContentWidth - screenshotSize.width;
      if (screenshotSize.height < targetContentHeight) physicalHeight += targetContentHeight - screenshotSize.height;
    }
    throw new Error(`Unable to establish a 1536x${evidenceHeight} WebView content snapshot; attempts ${JSON.stringify(attempts)}`);
  },
  onComplete: () => {
    fixture.cleanup();
  },
};

export { fixtureRoot, outputDirectory, target };
