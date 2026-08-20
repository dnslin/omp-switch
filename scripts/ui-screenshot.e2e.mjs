import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setScenario } from "./ui-fixture.mjs";

const outputDirectory = resolve(process.env.OMP_SWITCH_UI_OUTPUT ?? ".artifacts/issue-16/ui-acceptance");

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
  if (viewport.width !== 1536 || viewport.height !== 1024) {
    throw new Error(`UI evidence requires an exact 1536x1024 CSS viewport; received ${viewport.width}x${viewport.height} at dpr ${viewport.dpr}`);
  }
  const rawScreenshot = await browser.takeScreenshot();
  const screenshot = typeof rawScreenshot === "string" ? rawScreenshot : rawScreenshot.value;
  const rawResized = await browser.executeAsync((source, expectedWidth, expectedHeight, done) => {
    const image = new Image();
    image.onload = () => {
      if (image.naturalWidth !== expectedWidth || image.naturalHeight !== expectedHeight) {
        done({ error: `screenshot-size-${image.naturalWidth}x${image.naturalHeight}-expected-${expectedWidth}x${expectedHeight}` });
        return;
      }
      const canvas = document.createElement("canvas");
      canvas.width = 1536;
      canvas.height = 1024;
      const context = canvas.getContext("2d");
      if (!context) {
        done({ error: "canvas-context-unavailable" });
        return;
      }
      context.drawImage(image, 0, 0, 1536, 1024);
      done({ png: canvas.toDataURL("image/png").slice("data:image/png;base64,".length) });
    };
    image.onerror = () => done({ error: "screenshot-decode-failed" });
    image.src = `data:image/png;base64,${source}`;
  }, screenshot, Math.round(1536 * viewport.dpr), Math.round(1024 * viewport.dpr));
  const resized = rawResized.value ?? rawResized;
  if (!resized.png) throw new Error(`Unable to downsample ${name}: ${resized.error ?? "unknown error"}`);
  await writeFile(join(outputDirectory, `${name}.png`), Buffer.from(resized.png, "base64"));
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
async function setModelTestResult(result) {
  await browser.tauri.execute(
    (tauri, nextResult) => tauri.core.invoke("set_webdriver_model_test_state", { result: nextResult }),
    result,
  );
  await browser.refresh();
}

describe("OMP Switch real packaged UI evidence", () => {
  it("captures the nine approved states at 1536x1024", async () => {
    await mkdir(outputDirectory, { recursive: true });
    const rawViewport = await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio }));
    const viewport = rawViewport.value ?? rawViewport;
    if (viewport.width !== 1536 || viewport.height !== 1024) {
      throw new Error(`UI evidence requires an exact 1536x1024 CSS viewport; received ${viewport.width}x${viewport.height} at dpr ${viewport.dpr}`);
    }

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
    await screenshot("provider-detail");

    setScenario("roles");
    await (await visible('a[href="/roles"]')).click();
    await waitHeading("角色");
    await selectOption("Thinking advisor", "max");
    await visible(`//*[contains(normalize-space(), ${xpathLiteral("有未保存的修改")})]`);
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
    await screenshot("provider-create-step1");

    await clickText("下一步", "button");
    await visible('[role="dialog"]');
    await fill("#provider-model-id", "gpt-5.6-sol");
    await fill("#provider-model-name", "GPT 5.6 Sol");
    await browser.execute(() => {
      const activeElement = document.activeElement;
      if (activeElement instanceof HTMLElement) activeElement.blur();
    });
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
    await screenshot("model-create-sheet");
  });
});
