import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const fakeApiKey = "webdriver-fixture-key";
const detailModelSpecs = [
  ["gpt-5.5", "openai-responses", ["text", "image"], false, 200000, 128000],
  ["gpt-5.6-sol", "openai-responses", ["text", "image"], true, 200000, 128000],
  ["gpt-5.6-luna", "openai-responses", ["text", "image"], true, 200000, 128000],
  ["gpt-5.6-terra", "openai-responses", ["text", "image"], true, 200000, 128000],
  ["gpt-5.3-codex-spark", "openai-responses", ["text"], true, 200000, 128000],
  ["gemini-3.6-flash-high", "openai-responses", ["text", "image"], false, 200000, 128000],
  ["k3", "anthropic-messages", ["text"], false, 200000, 128000],
  ["niko/claude-opus-5", "anthropic-messages", ["text"], false, 200000, 128000],
];
const overviewModelSpecs = detailModelSpecs.map(([id, api, input, reasoning, contextWindow, maxTokens]) => (
  id === "gpt-5.6-sol" ? [id, api, input, reasoning, 356000, 32768] : [id, api, input, reasoning, contextWindow, maxTokens]
));
const googleModelSpecs = [["gemini-3.6-pro", "google-generative-ai", ["text", "image"], true, 1000000, 128000]];

const builtinRoles = {
  default: "dnslin/gpt-5.6-luna",
  smol: "dnslin/gpt-5.6-luna:off",
  slow: "dnslin/gpt-5.6-luna:minimal",
  vision: "dnslin/gpt-5.6-luna:low",
  plan: "dnslin/gpt-5.6-luna:medium",
  designer: "dnslin/gpt-5.6-luna:high",
  commit: "dnslin/gpt-5.6-luna:xhigh",
  tiny: "dnslin/gpt-5.6-luna:max",
  task: "dnslin/gpt-5.6-terra:xhigh",
  advisor: "dnslin/gpt-5.6-sol:medium",
};

const scenarioProviders = {
  overview: [
    { id: "dnslin", baseUrl: "https://cpa.example.xyz/v1", models: overviewModelSpecs },
    { id: "packyapi", baseUrl: "https://www.example.ai/v1", defaultApi: "openai-responses", models: detailModelSpecs.slice(0, 2) },
    { id: "fixture-alpha", baseUrl: "https://alpha.example.xyz/v1", defaultApi: "openai-responses", models: detailModelSpecs.slice(0, 1) },
    { id: "fixture-beta", baseUrl: "https://beta.example.xyz/v1", defaultApi: "google-generative-ai", models: googleModelSpecs },
  ],
  providers: [
    { id: "dnslin", baseUrl: "https://cpa.example.xyz/v1", models: detailModelSpecs },
    { id: "packyapi", baseUrl: "https://www.example.ai/v1", defaultApi: "openai-responses", models: detailModelSpecs.slice(0, 3) },
  ],
  detail: [
    { id: "dnslin", baseUrl: "https://cpa.example.xyz/v1", models: detailModelSpecs },
  ],
  roles: [
    { id: "dnslin", baseUrl: "https://cpa.example.xyz/v1", models: detailModelSpecs },
  ],
};

const scenarioRoles = {
  overview: { ...builtinRoles, advisor: "dnslin/gpt-5.6-sol:max" },
  providers: { ...builtinRoles, advisor: "dnslin/gpt-5.6-sol:max" },
  detail: {},
  roles: { ...builtinRoles, researcher: "dnslin/gpt-5.6-luna:auto" },
};

function fixtureRoot() {
  const root = process.env.OMP_SWITCH_UI_FIXTURE_ROOT;
  if (!root) throw new Error("OMP_SWITCH_UI_FIXTURE_ROOT is not set");
  return resolve(root);
}

function providerYaml(provider) {
  const lines = [
    `  ${provider.id}:`,
    `    baseUrl: ${provider.baseUrl}`,
    provider.defaultApi ? `    api: ${provider.defaultApi}` : null,
    `    apiKey: ${fakeApiKey}`,
    "    models:",
  ].filter(Boolean);
  for (const [id, api, input, reasoning, contextWindow, maxTokens] of provider.models) {
    lines.push(`      - id: ${id}`);
    lines.push(`        name: ${id}`);
    lines.push(`        api: ${api}`);
    lines.push(`        input: [${input.join(", ")}]`);
    lines.push(`        reasoning: ${reasoning}`);
    lines.push(`        contextWindow: ${contextWindow}`);
    lines.push(`        maxTokens: ${maxTokens}`);
  }
  return lines.join("\n");
}

function modelsYaml(providers) {
  return `providers:\n${providers.map(providerYaml).join("\n")}\n`;
}

function rolesYaml(roles) {
  const entries = Object.entries(roles);
  if (entries.length === 0) return "modelRoles: {}\n";
  return `modelRoles:\n${entries.map(([id, selector]) => `  ${id}: ${selector}`).join("\n")}\n`;
}

export function setScenario(name) {
  const root = fixtureRoot();
  const providers = scenarioProviders[name];
  if (!providers) throw new Error(`Unknown UI evidence scenario: ${name}`);
  writeFileSync(join(root, "agent", "models.yml"), modelsYaml(providers), "utf8");
  writeFileSync(join(root, "agent", "config.yml"), rolesYaml(scenarioRoles[name]), "utf8");
}

export function writeSettings() {
  const root = fixtureRoot();
  const settings = JSON.stringify({
    ompExecutablePath: null,
    theme: "light",
    selectedProviderId: "dnslin",
    selectedModelId: "gpt-5.6-sol",
    modelTestCostNoticeAccepted: true,
  }, null, 2);
  const candidates = [
    join(root, "Library", "Application Support", "app.ompswitch.desktop", "settings.json"),
    join(root, "data", "app.ompswitch.desktop", "settings.json"),
    join(root, "appdata", "app.ompswitch.desktop", "settings.json"),
    join(root, "localappdata", "app.ompswitch.desktop", "settings.json"),
  ];
  for (const path of candidates) {
    mkdirSync(resolve(path, ".."), { recursive: true });
    writeFileSync(path, settings, "utf8");
  }
}

export function createUiFixture() {
  const existingRoot = process.env.OMP_SWITCH_UI_FIXTURE_ROOT;
  const root = existingRoot ? resolve(existingRoot) : mkdtempSync(join(tmpdir(), "omp-switch-ui-acceptance-"));
  process.env.OMP_SWITCH_UI_FIXTURE_ROOT = root;
  const target = join(root, "agent");
  const bin = join(root, "bin");
  mkdirSync(target, { recursive: true });
  mkdirSync(bin, { recursive: true });
  const targetPathForOmp = process.platform === "win32" ? target.replaceAll("/", "\\") : target;
  if (process.platform === "win32") {
    writeFileSync(join(bin, "omp.cmd"), `@echo off\nif "%~1"=="--version" echo 17.2.15\nif "%~1"=="config" if "%~2"=="path" echo %OMP_SWITCH_UI_TARGET%\n`, "utf8");
  } else {
    const executable = join(bin, "omp");
    writeFileSync(executable, `#!/bin/sh\nif [ "$1" = "--version" ]; then printf '17.2.15\\n'; exit 0; fi\nif [ "$1" = "config" ] && [ "$2" = "path" ]; then printf '%s\\n' "$OMP_SWITCH_UI_TARGET"; exit 0; fi\nexit 1\n`, "utf8");
    chmodSync(executable, 0o755);
  }
  process.env.PATH = `${bin}${process.platform === "win32" ? ";" : ":"}${process.env.PATH ?? ""}`;
  process.env.OMP_SWITCH_UI_TARGET = targetPathForOmp;
  process.env.TAURI_WEBDRIVER_PORT = String(Number(process.env.TAURI_WEBDRIVER_PORT ?? 4445));
  if (process.platform === "win32") {
    process.env.APPDATA = join(root, "appdata");
    process.env.LOCALAPPDATA = join(root, "localappdata");
    process.env.USERPROFILE = root;
    process.env.HOMEDRIVE = root.slice(0, 2);
    process.env.HOMEPATH = root.slice(2);
  } else {
    process.env.HOME = root;
    process.env.XDG_DATA_HOME = join(root, "data");
    process.env.XDG_CONFIG_HOME = join(root, "config");
    process.env.XDG_CACHE_HOME = join(root, "cache");
  }
  return {
    root,
    target,
    bin,
    cleanup: () => {
      if (!existingRoot) rmSync(root, { recursive: true, force: true });
    },
  };
}
