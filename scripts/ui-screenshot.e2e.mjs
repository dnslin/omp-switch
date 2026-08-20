import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setScenario } from "./ui-fixture.mjs";

const outputDirectory = resolve(process.env.OMP_SWITCH_UI_OUTPUT ?? ".artifacts/issue-16/ui-acceptance");
const evidenceHeight = Number(process.env.OMP_SWITCH_UI_HEIGHT ?? "960");
if (![960, 1024].includes(evidenceHeight)) throw new Error(`OMP_SWITCH_UI_HEIGHT must be 960 or 1024; received ${evidenceHeight}`);
function xpathLiteral(value) {
  if (!value.includes("'")) return `'${value}'`;
  if (!value.includes('"')) return `"${value}"`;
  return `concat(${value.split("'").map((part, index) => `${index ? `, "'", ` : ""}'${part}'`).join("")})`;
}
function xpath(text, tag = "*") {
  return `//${tag}[normalize-space()=${xpathLiteral(text)}]`;
}
async function visible(selector) {
  const element = await $(selector);
  await element.waitForDisplayed();
  return element;
}
async function clickText(text, tag = "*") {
  await (await visible(xpath(text, tag))).click();
}
async function waitHeading(text) {
  await visible(xpath(text, "h1"));
  await browser.pause(250);
}
async function screenshot(name) {
  await browser.pause(350);
  const rawViewport = await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio }));
  const viewport = rawViewport.value ?? rawViewport;
  if (viewport.width !== 1536 || viewport.height !== evidenceHeight) {
    throw new Error(`UI evidence requires a 1536x${evidenceHeight} WebView content viewport; received inner ${viewport.width}x${viewport.height} at dpr ${viewport.dpr}`);
  }
  const rawScreenshot = await browser.takeScreenshot();
  const screenshot = typeof rawScreenshot === "string" ? rawScreenshot : rawScreenshot.value;
  if (!Number.isFinite(viewport.dpr) || viewport.dpr <= 0) {
    throw new Error(`UI evidence requires a valid devicePixelRatio; received ${viewport.dpr}`);
  }
  const expectedWidth = Math.round(1536 * viewport.dpr);
  const expectedHeight = Math.round(evidenceHeight * viewport.dpr);
  const normalized = await browser.executeAsync((source, expectedWidth, expectedHeight, outputHeight, done) => {
    const image = new Image();
    image.onload = () => {
      if (image.naturalWidth !== expectedWidth || image.naturalHeight < expectedHeight) {
        done({ error: `screenshot-size-${image.naturalWidth}x${image.naturalHeight}-expected-width-${expectedWidth}-minimum-height-${expectedHeight}` });
        return;
      }
      const sourceCanvas = document.createElement("canvas");
      sourceCanvas.width = image.naturalWidth;
      sourceCanvas.height = image.naturalHeight;
      const sourceContext = sourceCanvas.getContext("2d");
      if (!sourceContext) {
        done({ error: "source-canvas-context-unavailable" });
        return;
      }
      sourceContext.drawImage(image, 0, 0);
      const extraHeight = image.naturalHeight - expectedHeight;
      let excludedBandColor = null;
      if (extraHeight > 0) {
        const extra = sourceContext.getImageData(0, expectedHeight, image.naturalWidth, extraHeight).data;
        excludedBandColor = [extra[0], extra[1], extra[2], extra[3]];
        for (let index = 0; index < extra.length; index += 4) {
          if (extra[index] !== excludedBandColor[0] || extra[index + 1] !== excludedBandColor[1] || extra[index + 2] !== excludedBandColor[2] || extra[index + 3] !== excludedBandColor[3]) {
            done({ error: `unexpected-nonuniform-bottom-snapshot-band-${extraHeight}px` });
            return;
          }
        }
      }
      const canvas = document.createElement("canvas");
      canvas.width = 1536;
      canvas.height = outputHeight;
      const context = canvas.getContext("2d");
      if (!context) {
        done({ error: "canvas-context-unavailable" });
        return;
      }
      context.drawImage(image, 0, 0, expectedWidth, expectedHeight, 0, 0, 1536, outputHeight);
      done({
        png: canvas.toDataURL("image/png").slice("data:image/png;base64,".length),
        rawWidth: image.naturalWidth,
        rawHeight: image.naturalHeight,
        excludedBottomPixels: extraHeight,
        excludedBandColor,
      });
    };
    image.onerror = () => done({ error: "screenshot-decode-failed" });
    image.src = `data:image/png;base64,${source}`;
  }, screenshot, expectedWidth, expectedHeight, evidenceHeight);
  const result = normalized.value ?? normalized;
  if (!result.png) {
    await writeFile(join(outputDirectory, `${name}.raw.png`), Buffer.from(screenshot, "base64"));
    throw new Error(`Unable to normalize ${name}: ${result.error ?? "unknown error"}`);
  }
  await writeFile(join(outputDirectory, `${name}.normalization.json`), `${JSON.stringify({
    state: name,
    rawSnapshot: { width: result.rawWidth, height: result.rawHeight },
    output: { width: 1536, height: evidenceHeight },
    excludedBottomPixels: result.excludedBottomPixels,
    excludedBandColor: result.excludedBandColor,
  }, null, 2)}\n`);
  await writeFile(join(outputDirectory, `${name}.png`), Buffer.from(result.png, "base64"));
}
async function selectOption(label, optionText) {
  await (await visible(`[aria-label=${JSON.stringify(label)}]`)).click();
  await (await visible(`//*[@role="option" and normalize-space()=${xpathLiteral(optionText)}]`)).click();
}
async function fill(selector, value) {
  const element = await visible(selector);
  await element.clearValue();
  await element.setValue(value);
}
async function blurActiveElement() {
  await browser.execute(() => {
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement) activeElement.blur();
  });
}
async function setModelTestResult(result) {
  await browser.tauri.execute(
    (tauri, nextResult) => tauri.core.invoke("set_webdriver_model_test_state", { result: nextResult }),
    result,
  );
  await browser.refresh();
}
async function assertOverviewEndpointVisible() {
  const rawMetrics = await browser.execute(() => {
    const row = Array.from(document.querySelectorAll(".overview-result-row")).find((candidate) => candidate.firstElementChild?.textContent?.trim() === "最终地址");
    const value = row?.lastElementChild;
    const panel = row?.closest(".overview-result");
    if (!(value instanceof HTMLElement) || !(panel instanceof HTMLElement)) return { error: "overview-endpoint-row-not-found" };
    const range = document.createRange();
    range.selectNodeContents(value);
    const textRight = Math.max(...Array.from(range.getClientRects(), (rect) => rect.right));
    const textStyle = getComputedStyle(value);
    const panelRect = panel.getBoundingClientRect();
    return {
      text: value.textContent?.trim() ?? "",
      clientWidth: value.clientWidth,
      scrollWidth: value.scrollWidth,
      textRight,
      panelRight: panelRect.right,
      overflow: textStyle.overflow,
      textOverflow: textStyle.textOverflow,
    };
  });
  const metrics = rawMetrics.value ?? rawMetrics;
  if (metrics.error) throw new Error(metrics.error);
  if (metrics.text !== "https://cpa.example.xyz/v1/responses") {
    throw new Error(`overview endpoint text mismatch: ${JSON.stringify(metrics.text)}`);
  }
  const fitsInsideCell = metrics.scrollWidth <= metrics.clientWidth + 1;
  const visiblyFitsInsidePanel = metrics.overflow === "visible" && metrics.textOverflow === "clip" && metrics.textRight <= metrics.panelRight + 1;
  if (!fitsInsideCell && !visiblyFitsInsidePanel) {
    throw new Error(`overview endpoint is clipped: scrollWidth=${metrics.scrollWidth}, clientWidth=${metrics.clientWidth}, textRight=${metrics.textRight}, panelRight=${metrics.panelRight}, overflow=${metrics.overflow}, textOverflow=${metrics.textOverflow}`);
  }
}

describe("OMP Switch real packaged UI evidence", () => {
  it(`captures the nine approved 1536x${evidenceHeight} states`, async () => {
    await mkdir(outputDirectory, { recursive: true });

    await waitHeading("OMP 已找到");
    await screenshot("setup-success");

    await clickText("进入应用", "button");
    await waitHeading("概览");
    await selectOption("Provider", "dnslin");
    await selectOption("模型", "gpt-5.6-sol");
    await setModelTestResult({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses",
      latencyMs: 842,
      status: 200,
      message: "测试成功",
    });
    await waitHeading("概览");
    await assertOverviewEndpointVisible();
    await screenshot("overview");

    setScenario("providers");
    await (await visible('a[href="/providers"]')).click();
    await waitHeading("Providers");
    await screenshot("providers-list");

    setScenario("detail");
    await (await visible('a[href="/providers/dnslin"]')).click();
    await waitHeading("dnslin");
    await setModelTestResult({
      success: true,
      providerId: "dnslin",
      modelId: "k3",
      protocol: "anthropic-messages",
      latencyMs: 842,
      status: 200,
      message: "测试成功",
    });
    await waitHeading("dnslin");
    await blurActiveElement();
    await screenshot("provider-detail");
    setScenario("roles");
    await (await visible('a[href="/roles"]')).click();
    await waitHeading("角色");
    await selectOption("Thinking advisor", "max");
    await visible(`//*[contains(normalize-space(), ${xpathLiteral("有未保存的修改")})]`);
    await blurActiveElement();
    await screenshot("roles-dirty");

    await (await visible('a[href="/settings"]')).click();
    await visible('[role="dialog"]');
    await clickText("放弃修改", "button");
    await waitHeading("设置");
    await screenshot("settings");

    setScenario("providers");
    await (await visible('a[href="/providers"]')).click();
    await waitHeading("Providers");
    await clickText("新增 Provider", "button");
    await visible('[role="dialog"]');
    await fill("#provider-id", "dnslin");
    await fill("#provider-base-url", "https://cpa.example.xyz/v1");
    await fill("#provider-api-key", `sk-${"x".repeat(40)}`);
    await blurActiveElement();
    await screenshot("provider-create-step1");

    await clickText("下一步", "button");
    await visible('[role="dialog"]');
    await fill("#provider-model-id", "gpt-5.6-sol");
    await fill("#provider-model-name", "GPT 5.6 Sol");
    await blurActiveElement();
    await screenshot("provider-create-step2");

    await clickText("取消", "button");
    await clickText("放弃修改", "button");
    await waitHeading("Providers");
    setScenario("detail");
    await (await visible('a[href="/providers/dnslin"]')).click();
    await waitHeading("dnslin");
    await clickText("新增模型", "button");
    await visible('[role="dialog"]');
    await fill("#model-sheet-id", "gemini-3.6-pro");
    await fill("#model-sheet-name", "Gemini 3.6 Pro");
    await selectOption("协议", "google-generative-ai");
    await blurActiveElement();
    await screenshot("model-create-sheet");
  });
});
