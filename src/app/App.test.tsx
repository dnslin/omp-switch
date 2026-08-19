import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { TauriClientProvider, type OverviewDto, type OverviewLoad, type OverviewModel, type OverviewProvider, type StartupState, type TargetConfigurationDiscovery, type TauriClient } from "../lib/tauri-client";
import { useModelTestStore } from "../store/model-test";


function targetConfiguration(
  path = "/Users/username/.omp/agent",
  overrides: Partial<TargetConfigurationDiscovery> = {},
): TargetConfigurationDiscovery {
  const file = (name: string) => ({
    canonicalPath: `${path}/${name}`,
    resolvedPath: `${path}/${name}`,
    status: "normal" as const,
  });
  return {
    path,
    resolvedPath: path,
    status: "writable",
    writable: true,
    models: file("models.yml"),
    config: file("config.yml"),
    recoveryNotice: null,
    createPaths: [],
    discoveryToken: `discovery:${path}`,
    warnings: [],
    issue: null,
    ...overrides,
  };
}
const unavailableClient: TauriClient = {
  getStartupState: async () => ({
    kind: "omp-unavailable",
    message: "未在已保存路径或系统 PATH 中找到可用的 OMP。",
  }),
  getOverviewLoad: async () => overviewLoad(overviewDto({ state: "empty", counts: { providerCount: 0, modelCount: 0, roleCount: 0 }, providers: [], models: [], roles: [], emptyReason: "还没有可管理的自定义 Provider。", nextAction: "创建一个 Provider，并同时配置它的第一个模型。" })),
  createCustomProvider: async () => ({ providerId: "new-provider", modelId: "new-model" }),
  editCustomProvider: async () => ({ providerId: "dnslin" }),
  createModel: async () => ({ providerId: "dnslin", modelId: "new-model" }),
  editModel: async () => ({ providerId: "dnslin", modelId: "gpt-5.6-sol" }),
  deleteModel: async () => ({ providerId: "dnslin", modelId: "gpt-5.6-sol" }),
  deleteProvider: async () => ({ providerId: "dnslin", modelCount: 1 }),
  saveModelRoles: async () => ({ changedRoleCount: 0 }),
  testModel: async () => ({ success: true, providerId: "dnslin", modelId: "gpt-5.6-sol", protocol: "openai-responses" as const, latencyMs: 12, status: 200, message: "模型连接成功" }),
  cancelModelTest: async () => true,
  getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: null, terminal: null }),
  detectOmp: async () => ({ kind: "omp-unavailable", message: "仍未找到 OMP" }),
  selectOmpExecutable: async () => null,
  validateSelectedOmp: async () => ({ kind: "invalid-executable", executablePath: "/tmp/not-omp", message: "无法运行", diagnosticCode: "io-not-found" }),
  confirmSelectedOmp: async () => undefined,
  initializeTargetConfiguration: async () => readyState,
  openTargetConfigurationDirectory: async () => undefined,
  getUiSettings: async () => ({
    ompExecutablePath: null,
    theme: "system",
    selectedProviderId: null,
    selectedModelId: null,
    modelTestCostNoticeAccepted: false,
  }),
  saveUiSettings: async (settings) => ({ ompExecutablePath: null, modelTestCostNoticeAccepted: false, ...settings }),
  acceptModelTestCostNotice: async () => ({ ompExecutablePath: null, theme: "system", selectedProviderId: null, selectedModelId: null, modelTestCostNoticeAccepted: true }),
};
const readyState: StartupState = {
  kind: "omp-ready",
  executablePath: "/usr/local/bin/omp",
  version: "17.4.1",
  targetConfiguration: targetConfiguration(),
  previousTargetConfiguration: null,
  requiresConfirmation: false,
};

function overviewLoad(overview: OverviewDto, startupState: StartupState = { kind: "omp-unavailable", message: "未在已保存路径或系统 PATH 中找到可用的 OMP。" }): OverviewLoad {
  return { startupState, overview, error: null };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}


function renderRoute(route: string, client: TauriClient = unavailableClient, strictMode = false) {
  const router = createMemoryRouter([{ path: "*", element: <App /> }], { initialEntries: [route] });
  const app = (
    <TauriClientProvider client={client}>
      <RouterProvider router={router} />
    </TauriClientProvider>
  );
  return { ...render(strictMode ? <StrictMode>{app}</StrictMode> : app), router };
}

type ProviderWizardValues = {
  providerId: string;
  baseUrl: string;
  modelId: string;
  modelName: string;
};

async function fillProviderWizard(
  user: ReturnType<typeof userEvent.setup>,
  values: ProviderWizardValues,
) {
  await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
  await user.type(screen.getByLabelText("Provider ID"), values.providerId);
  await user.type(screen.getByLabelText("Base URL"), values.baseUrl);
  await user.click(screen.getByRole("button", { name: "下一步" }));
  await user.type(await screen.findByLabelText("Model ID"), values.modelId);
  await user.type(screen.getByLabelText("名称"), values.modelName);
}

describe("React page seam", () => {
  it("renders the not-found state and retries detection", async () => {
    const user = userEvent.setup();
    const detectOmp = vi.fn(unavailableClient.detectOmp);
    renderRoute("/setup", { ...unavailableClient, detectOmp });

    expect(await screen.findByRole("heading", { name: "设置 OMP" })).toBeVisible();
    expect(screen.getAllByText(/未在已保存路径或系统 PATH/)[0]).toBeVisible();
    await user.click(screen.getByRole("button", { name: "自动检测" }));
    expect(detectOmp).toHaveBeenCalledTimes(1);
    expect((await screen.findAllByText("仍未找到 OMP"))[0]).toBeVisible();
  });
  it("leaves startup detection failure actionable", async () => {
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => { throw new Error("invoke failed"); },
    });

    expect(await screen.findByRole("heading", { name: "设置 OMP" })).toBeVisible();
    expect(screen.getByRole("button", { name: "自动检测" })).toBeEnabled();
  });


  it.each([
    [{ kind: "detecting" } as const, "正在检测 OMP…"],
    [{ kind: "invalid-executable", executablePath: "/tmp/not-omp", message: "所选文件无法运行", diagnosticCode: "io-not-found" } as const, "所选文件无法运行"],
    [{ kind: "version-failed", executablePath: "/tmp/omp", message: "版本失败", diagnosticCode: "process-exit", exitCode: 7, stderr: "技术详情已脱敏" } as const, "版本失败"],
    [{ kind: "config-path-failed", executablePath: "/tmp/omp", version: "17.4.1", message: "不会猜测目录。该命令可能初始化 OMP Settings、访问 agent.db，或运行 OMP 自身的旧迁移。", diagnosticCode: "process-exit", exitCode: 9, stderr: "技术详情已脱敏" } as const, "不会猜测目录"],
  ])("renders setup state %#", async (startupState, visibleText) => {
    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => startupState });
    expect((await screen.findAllByText(new RegExp(visibleText)))[0]).toBeVisible();
  });

  it("matches the approved success content and enters the application", async () => {
    const user = userEvent.setup();
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => ({
        kind: "omp-ready",
        executablePath: "/usr/local/bin/omp",
        version: "17.4.1",
        targetConfiguration: targetConfiguration(),
        previousTargetConfiguration: null,
        requiresConfirmation: false,
      }),
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    expect(screen.getByText("OMP Switch 已确认可执行文件和权威配置目录。")).toBeVisible();
    expect(screen.getByText("/usr/local/bin/omp")).toBeVisible();
    expect(screen.getByText("/Users/username/.omp/agent")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "进入应用" }));
    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
  });
  it("shows an interrupted initialization recovery result", async () => {
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => ({
        ...readyState,
        targetConfiguration: targetConfiguration(undefined, {
          recoveryNotice: "已回滚上次中断的 Target configuration 初始化；未保留部分创建结果。",
        }),
      }),
    });

    expect(await screen.findByText("已恢复上次中断操作")).toBeVisible();
    expect(screen.getByText(/已回滚上次中断/)).toBeVisible();
  });


  it("allows selecting a replacement while the current OMP is ready", async () => {
    const user = userEvent.setup();
    const selectOmpExecutable = vi.fn(async () => "/opt/new/bin/omp");
    const replacementState: StartupState = {
      ...readyState,
      executablePath: "/opt/new/bin/omp",
      version: "18.0.0",
      targetConfiguration: targetConfiguration("/Users/username/.omp/new-agent"),
      previousTargetConfiguration: readyState.targetConfiguration.path,
      requiresConfirmation: true,
    };
    const validateSelectedOmp = vi.fn(async () => replacementState);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      selectOmpExecutable,
      validateSelectedOmp,
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "手动选择 OMP" }));

    expect(selectOmpExecutable).toHaveBeenCalledTimes(1);
    expect(validateSelectedOmp).toHaveBeenCalledWith("/opt/new/bin/omp");
    expect(await screen.findByRole("heading", { name: "确认切换 OMP" })).toBeVisible();
  });

  it("keeps the successful setup layout mounted while redetection is pending", async () => {
    const user = userEvent.setup();
    let resolveDetection!: (state: StartupState) => void;
    const detectOmp = vi.fn(() => new Promise<StartupState>((resolve) => { resolveDetection = resolve; }));
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      detectOmp,
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "重新检测" }));

    const retryButton = screen.getByRole("button", { name: "重新检测" });
    const enterButton = screen.getByRole("button", { name: "进入应用" });
    expect(screen.getByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    expect(screen.getByText("/Users/username/.omp/agent")).toBeVisible();
    expect(retryButton).toBeDisabled();
    expect(screen.getByTestId("redetect-progress")).toHaveClass("redetect-overlay");
    expect(screen.getByTestId("redetect-progress")).toHaveTextContent("正在重新检测 OMP");
    expect(enterButton).toBeDisabled();
    expect(enterButton).toHaveClass("app-button--disabled-stable");

    await screen.findByTestId("redetect-progress");
    resolveDetection({ kind: "omp-unavailable", message: "仍未找到 OMP" });
    await waitFor(() => expect(screen.queryByTestId("redetect-progress")).not.toBeInTheDocument(), { timeout: 2000 });
    expect(screen.getAllByText("仍未找到 OMP")[0]).toBeVisible();
  });
  it("rejects duplicate detection starts before React commits disabled state", async () => {
    let resolveDetection!: (state: StartupState) => void;
    const detectOmp = vi.fn(() => new Promise<StartupState>((resolve) => { resolveDetection = resolve; }));
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      detectOmp,
    });

    const retryButton = await screen.findByRole("button", { name: "重新检测" });
    retryButton.click();
    retryButton.click();

    expect(detectOmp).toHaveBeenCalledTimes(1);
    resolveDetection(readyState);
    await waitFor(() => expect(retryButton).toBeEnabled(), { timeout: 2000 });
  });


  it("keeps fast unchanged redetection visually stable with Dot Matrix feedback", async () => {
    const detectOmp = vi.fn(async () => readyState);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      detectOmp,
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    vi.useFakeTimers();
    try {
      const retryButton = screen.getByRole("button", { name: "重新检测" });
      fireEvent.click(retryButton);

      expect(detectOmp).toHaveBeenCalledTimes(1);
      expect(retryButton).toBeDisabled();
      expect(screen.getByTestId("redetect-progress")).toHaveClass("redetect-overlay");
      expect(screen.getByTestId("redetection-loader").children).toHaveLength(25);
      expect(screen.getByTestId("redetect-progress").firstElementChild).toHaveClass("redetect-overlay__content");
      expect(screen.getByText("/Users/username/.omp/agent")).toBeVisible();
      await act(async () => { await vi.advanceTimersByTimeAsync(1199); });
      expect(retryButton).toBeDisabled();
      expect(screen.getByTestId("redetect-progress")).toBeInTheDocument();

      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(retryButton).toBeEnabled();
      expect(screen.queryByTestId("redetect-progress")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps fast failed redetection feedback visible for the minimum duration", async () => {
    const detectOmp = vi.fn(async () => { throw new Error("检测失败"); });
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      detectOmp,
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    vi.useFakeTimers();
    try {
      fireEvent.click(screen.getByRole("button", { name: "重新检测" }));

      expect(screen.getByTestId("redetect-progress")).toBeInTheDocument();
      await act(async () => { await vi.advanceTimersByTimeAsync(1199); });
      expect(screen.getByTestId("redetect-progress")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "OMP 已找到" })).toBeVisible();

      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(screen.queryByTestId("redetect-progress")).not.toBeInTheDocument();
      expect(screen.getAllByText("无法重新检测 OMP")[0]).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("lists full paths and confirms atomic Target configuration creation", async () => {
    const user = userEvent.setup();
    const creationState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "creation-required",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "missing", resolvedPath: null },
        config: { ...readyState.targetConfiguration.config, status: "missing", resolvedPath: null },
        createPaths: [
          readyState.targetConfiguration.path,
          readyState.targetConfiguration.models.canonicalPath,
          readyState.targetConfiguration.config.canonicalPath,
        ],
      }),
    };
    const initializeTargetConfiguration = vi.fn(async () => readyState);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => creationState,
      initializeTargetConfiguration,
    });

    expect(await screen.findByRole("heading", { name: "需要创建 OMP 配置" })).toBeVisible();
    for (const path of creationState.targetConfiguration.createPaths) {
      expect(screen.getAllByText(path)[0]).toBeVisible();
    }
    expect(screen.queryByRole("button", { name: "进入应用" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "创建" }));
    expect(initializeTargetConfiguration).toHaveBeenCalledWith("/usr/local/bin/omp", {
      createPaths: creationState.targetConfiguration.createPaths,
      discoveryToken: creationState.targetConfiguration.discoveryToken,
    });
    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    expect(screen.getByRole("button", { name: "进入应用" })).toBeEnabled();
  });

  it("confirms an OMP switch before creating its target configuration", async () => {
    const user = userEvent.setup();
    const creationState: StartupState = {
      ...readyState,
      requiresConfirmation: true,
      previousTargetConfiguration: "/Users/username/.omp/old-agent",
      targetConfiguration: targetConfiguration("/Users/username/.omp/new-agent", {
        status: "creation-required",
        writable: false,
        resolvedPath: null,
        models: { canonicalPath: "/Users/username/.omp/new-agent/models.yml", resolvedPath: null, status: "missing" },
        config: { canonicalPath: "/Users/username/.omp/new-agent/config.yml", resolvedPath: null, status: "missing" },
        createPaths: ["/Users/username/.omp/new-agent/models.yml", "/Users/username/.omp/new-agent/config.yml"],
      }),
    };
    const initializeTargetConfiguration = vi.fn(async () => readyState);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => creationState,
      initializeTargetConfiguration,
    });

    await user.click(await screen.findByRole("button", { name: "确认切换并创建" }));

    expect(initializeTargetConfiguration).toHaveBeenCalledWith("/usr/local/bin/omp", {
      createPaths: creationState.targetConfiguration.createPaths,
      discoveryToken: creationState.targetConfiguration.discoveryToken,
    });
    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
  });
  it("refreshes the pending OMP after initialization fails", async () => {
    const user = userEvent.setup();
    const creationState: StartupState = {
      ...readyState,
      requiresConfirmation: true,
      previousTargetConfiguration: "/Users/username/.omp/old-agent",
      targetConfiguration: targetConfiguration("/Users/username/.omp/new-agent", {
        status: "creation-required",
        writable: false,
        resolvedPath: null,
        models: { canonicalPath: "/Users/username/.omp/new-agent/models.yml", resolvedPath: null, status: "missing" },
        config: { canonicalPath: "/Users/username/.omp/new-agent/config.yml", resolvedPath: null, status: "missing" },
        createPaths: ["/Users/username/.omp/new-agent/models.yml", "/Users/username/.omp/new-agent/config.yml"],
      }),
    };
    const initializeTargetConfiguration = vi.fn(async () => {
      throw { code: "target-initialization-failed", message: "创建失败", action: "原 OMP 选择已恢复" };
    });
    const validateSelectedOmp = vi.fn(async () => creationState);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => creationState,
      initializeTargetConfiguration,
      validateSelectedOmp,
    });

    await user.click(await screen.findByRole("button", { name: "确认切换并创建" }));

    expect(validateSelectedOmp).toHaveBeenCalledWith("/usr/local/bin/omp");
    expect(await screen.findByRole("heading", { name: "需要创建 OMP 配置" })).toBeVisible();
    expect(screen.getByRole("button", { name: "确认切换并创建" })).toBeEnabled();
  });

  it("offers creation instead of overview when alternate yaml is mixed with a missing canonical file", async () => {
    const mixedState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "creation-required",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "alternate-only", resolvedPath: "/Users/username/.omp/agent/models.yaml" },
        config: { ...readyState.targetConfiguration.config, status: "missing", resolvedPath: null },
        createPaths: [readyState.targetConfiguration.config.canonicalPath],
      }),
    };

    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => mixedState });

    expect(await screen.findByRole("heading", { name: "需要创建 OMP 配置" })).toBeVisible();
    expect(screen.getByRole("button", { name: "创建" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /进入/ })).not.toBeInTheDocument();
  });


  it("enters read-only mode for yaml-only configuration without creating yml", async () => {
    const user = userEvent.setup();
    const yamlOnlyState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "read-only",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "alternate-only", resolvedPath: "/Users/username/.omp/agent/models.yaml" },
        config: { ...readyState.targetConfiguration.config, status: "alternate-only", resolvedPath: "/Users/username/.omp/agent/config.yaml" },
      }),
    };
    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => yamlOnlyState });

    expect(await screen.findByRole("heading", { name: "当前配置使用 .yaml" })).toBeVisible();
    expect(screen.getByText(/MVP 只写入 .yml/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "创建" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "打开配置目录" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "进入只读模式" }));
    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
  });

  it("requires official OMP migration for legacy JSON", async () => {
    const migrationState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "migration-required",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "legacy-json", resolvedPath: null },
        config: { ...readyState.targetConfiguration.config, status: "legacy-json", resolvedPath: null },
      }),
    };
    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => migrationState });

    expect(await screen.findByRole("heading", { name: "需要先由 OMP 迁移配置" })).toBeVisible();
    expect(screen.getByText(/官方 YAML 迁移/)).toBeVisible();
    expect(screen.queryByRole("button", { name: /进入/ })).not.toBeInTheDocument();
  });

  it("shows YAML location and external repair actions", async () => {
    const user = userEvent.setup();
    const openTargetConfigurationDirectory = vi.fn(async () => undefined);
    const detectOmp = vi.fn(async () => readyState);
    const parseState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "parse-error",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "parse-error" },
        issue: {
          filePath: "/Users/username/.omp/agent/models.yml",
          line: 18,
          column: 7,
          message: "did not find expected node content",
        },
      }),
    };
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => parseState,
      detectOmp,
      openTargetConfigurationDirectory,
    });

    expect(await screen.findByRole("heading", { name: "无法读取 models.yml" })).toBeVisible();
    expect(screen.getByText(/第 18 行，第 7 列/)).toBeVisible();
    expect(screen.getByText("did not find expected node content")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "打开配置目录" }));
    expect(openTargetConfigurationDirectory).toHaveBeenCalledWith("/usr/local/bin/omp");
    expect(screen.queryByRole("button", { name: "创建" })).not.toBeInTheDocument();
    vi.useFakeTimers();
    try {
      fireEvent.click(screen.getByRole("button", { name: "重新读取" }));
      expect(detectOmp).toHaveBeenCalledTimes(1);
      await act(async () => { await vi.advanceTimersByTimeAsync(1200); });
      expect(screen.getByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("revalidates the pending OMP after external parse repair", async () => {
    const pendingParse: StartupState = {
      ...readyState,
      requiresConfirmation: true,
      previousTargetConfiguration: "/Users/username/.omp/old-agent",
      targetConfiguration: targetConfiguration(undefined, {
        status: "parse-error",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "parse-error" },
        issue: { filePath: "/Users/username/.omp/agent/models.yml", line: 2, column: 4, message: "invalid YAML" },
      }),
    };
    const repaired: StartupState = { ...readyState, requiresConfirmation: true, previousTargetConfiguration: "/Users/username/.omp/old-agent" };
    const detectOmp = vi.fn(async () => readyState);
    const validateSelectedOmp = vi.fn(async () => repaired);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => pendingParse,
      detectOmp,
      validateSelectedOmp,
    });

    expect(await screen.findByRole("heading", { name: "无法读取 models.yml" })).toBeVisible();
    vi.useFakeTimers();
    try {
      fireEvent.click(screen.getByRole("button", { name: "重新读取" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(1200); });
      expect(validateSelectedOmp).toHaveBeenCalledWith("/usr/local/bin/omp");
      expect(detectOmp).not.toHaveBeenCalled();
      expect(screen.getByRole("heading", { name: "确认切换 OMP" })).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });


  it("shows untouched yaml warnings beside writable canonical files", async () => {
    const warningState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        models: { ...readyState.targetConfiguration.models, status: "canonical-with-alternate" },
        warnings: ["检测到 models.yaml；OMP Switch 使用 models.yml，且 models.yaml 不会被修改。"],
      }),
    };
    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => warningState });

    expect(await screen.findByText(/models.yaml 不会被修改/)).toBeVisible();
    expect(screen.getByRole("button", { name: "进入应用" })).toBeEnabled();
  });

  it("refuses entry when a link or reparse target is unsafe", async () => {
    const unsafeState: StartupState = {
      ...readyState,
      targetConfiguration: targetConfiguration(undefined, {
        status: "unsafe",
        writable: false,
        models: { ...readyState.targetConfiguration.models, status: "unsafe", resolvedPath: null },
        issue: {
          filePath: "/Users/username/.omp/agent/models.yml",
          line: null,
          column: null,
          message: "无法解析配置文件链接或重解析点：链接循环",
        },
      }),
    };
    renderRoute("/setup", { ...unavailableClient, getStartupState: async () => unsafeState });

    expect(await screen.findByRole("heading", { name: "无法安全访问 Target configuration" })).toBeVisible();
    expect(screen.getByText(/链接循环/)).toBeVisible();
    expect(screen.queryByRole("button", { name: /进入/ })).not.toBeInTheDocument();
  });

  it("shows and explicitly confirms the Target configuration change before switching OMP", async () => {
    const user = userEvent.setup();
    const confirmSelectedOmp = vi.fn(async () => undefined);
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => ({
        kind: "omp-ready",
        executablePath: "/opt/new/bin/omp",
        version: "18.0.0",
        targetConfiguration: targetConfiguration("/Users/username/.omp/new-agent"),
        previousTargetConfiguration: "/Users/username/.omp/old-agent",
        requiresConfirmation: true,
      }),
      confirmSelectedOmp,
    });

    expect(await screen.findByRole("heading", { name: "确认切换 OMP" })).toBeVisible();
    expect(screen.getByText("/Users/username/.omp/old-agent")).toBeVisible();
    expect(screen.getAllByText("/Users/username/.omp/new-agent")).toHaveLength(2);
    expect(confirmSelectedOmp).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "确认切换并进入应用" }));
    expect(confirmSelectedOmp).toHaveBeenCalledWith("/opt/new/bin/omp");
    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
  });
  it("redirects the root route through startup detection", async () => {
    renderRoute("/");

    expect(await screen.findByRole("heading", { name: "设置 OMP" })).toBeVisible();
  });

  it("opens the approved two-step Provider wizard and validates the first step before advancing", async () => {
    const user = userEvent.setup();
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    const create = await screen.findByRole("button", { name: "新增 Provider" });
    expect(create).toBeEnabled();
    await user.click(create);

    expect(screen.getByRole("dialog")).toBeVisible();
    expect(screen.getByRole("heading", { name: "新增 Provider" })).toBeVisible();
    expect(screen.getByText("步骤 1 / 2 · Provider")).toBeVisible();
    expect(screen.getByLabelText("Provider ID")).toBeVisible();
    expect(screen.getByLabelText("Base URL")).toBeVisible();
    expect(screen.getByRole("radiogroup", { name: "认证方式" })).toBeVisible();

    const next = screen.getByRole("button", { name: "下一步" });
    expect(next).toBeDisabled();
    await user.click(screen.getByLabelText("Provider ID"));
    await user.tab();
    await user.click(screen.getByLabelText("Base URL"));
    await user.tab();
    expect(await screen.findByText("Provider ID 不能为空。")).toBeVisible();
    expect(screen.getByText("Base URL 必须是有效的 HTTP 或 HTTPS 地址。")).toBeVisible();

    await user.type(screen.getByLabelText("Provider ID"), "new-provider");
    await user.type(screen.getByLabelText("Base URL"), "https://new-provider.example/v1");
    expect(screen.getByLabelText("Provider ID")).toHaveValue("new-provider");
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://new-provider.example/v1");
    expect(screen.getByRole("radio", { name: "API Key 认证" })).toBeChecked();
    expect(screen.getByRole("combobox", { name: "默认协议（可选）" })).toHaveTextContent("由模型指定");
    expect(next).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "下一步" }));
    expect(screen.queryByText("Provider ID 不能为空。")).not.toBeInTheDocument();
    expect(screen.queryByText("Base URL 必须是有效的 HTTP 或 HTTPS 地址。")).not.toBeInTheDocument();

    expect(await screen.findByText((_, element) => Boolean(
      element?.classList.contains("provider-create-step")
      && element.textContent === "步骤 2 / 2 · 首个模型",
    ))).toBeVisible();
    expect(screen.queryByText("Model ID 不能为空。")).not.toBeInTheDocument();
    expect(screen.queryByText("名称不能为空。")).not.toBeInTheDocument();
    expect(screen.getByText("new-provider")).toBeVisible();
    expect(screen.getByLabelText("Model ID")).toBeVisible();
    await user.click(screen.getByLabelText("Model ID"));
    await user.tab();
    expect(await screen.findByText("Model ID 不能为空。")).toBeVisible();
    expect(screen.queryByText("名称不能为空。")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Context Window")).toBeVisible();
    expect(screen.getByRole("button", { name: "创建 Provider" })).toBeDisabled();
  });

  it("submits a complete first model, reloads, and enters its Provider detail", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const createdModel: OverviewModel = {
      ...base.models[0],
      providerId: "new-provider",
      id: "new-model",
      name: "New Model",
      contextWindow: 356_000,
      maxTokens: 128_000,
    };
    const createdProvider: OverviewProvider = {
      ...base.providers[0],
      id: "new-provider",
      name: null,
      baseUrl: "https://new-provider.example/v1",
      authMode: "api-key",
      hasApiKey: false,
      modelCount: 1,
      models: [createdModel],
    };
    const initial = overviewDto({
      state: "empty",
      counts: { providerCount: 0, modelCount: 0, roleCount: 0 },
      providers: [],
      models: [],
      roles: [],
      emptyReason: "还没有可管理的自定义 Provider。",
      nextAction: "创建一个 Provider，并同时配置它的第一个模型。",
    });
    const created = overviewDto({
      counts: { providerCount: 1, modelCount: 1, roleCount: 0 },
      providers: [createdProvider],
      models: [createdModel],
      roles: [],
    });
    const getOverviewLoad = vi.fn()
      .mockResolvedValueOnce(overviewLoad(initial, readyState))
      .mockResolvedValue(overviewLoad(created, readyState));
    const createCustomProvider = vi.fn(async () => ({ providerId: "new-provider", modelId: "new-model" }));
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, createCustomProvider });

    await fillProviderWizard(user, {
      providerId: " new-provider ",
      baseUrl: "https://new-provider.example/v1/",
      modelId: " new-model ",
      modelName: "New Model",
    });
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "s", metaKey: true });

    await waitFor(() => expect(createCustomProvider).toHaveBeenCalledTimes(1));
    expect(createCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
      openedModelsHash: "models-hash",
      provider: expect.objectContaining({
        id: "new-provider",
        baseUrl: "https://new-provider.example/v1",
        authMode: "api-key",
      }),
      firstModel: expect.objectContaining({
        id: "new-model",
        name: "New Model",
        api: "openai-responses",
        input: ["text", "image"],
        contextWindow: 356_000,
        maxTokens: 128_000,
      }),
    }));
    expect(await screen.findByRole("heading", { name: "new-provider" })).toBeVisible();
    expect(screen.getByText("https://new-provider.example/v1")).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(3);
  });

  it("keeps the wizard open when post-create reload fails", async () => {
    const user = userEvent.setup();
    const refreshError = {
      code: "overview-read-failed",
      message: "无法重新读取 models.yml。",
      action: "请检查文件后重试。",
    };
    const getOverviewLoad = vi.fn()
      .mockResolvedValueOnce(overviewLoad(overviewDto(), readyState))
      .mockResolvedValueOnce({ startupState: readyState, overview: null, error: refreshError })
      .mockResolvedValue(overviewLoad(overviewDto(), readyState));
    const createCustomProvider = vi.fn(async () => ({ providerId: "new-provider", modelId: "new-model" }));
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, createCustomProvider });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.click(screen.getByRole("button", { name: "创建 Provider" }));

    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByText("Provider 已创建，但无法重新读取配置")).toBeVisible();
    expect(within(dialog).getByText("Provider 和首个模型已写入 models.yml。请重新读取以查看最新配置。")).toBeVisible();
    expect(within(dialog).getByText(refreshError.message)).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "创建 Provider" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "重新读取" }));

    await screen.findByRole("heading", { name: "Provider 不存在" });
    expect(getOverviewLoad).toHaveBeenCalledTimes(4);
  });

  it("submits spec-valid model IDs and token limits", async () => {
    const user = userEvent.setup();
    const createCustomProvider = vi.fn(async () => ({ providerId: "new-provider", modelId: "new-model:high" }));
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      createCustomProvider,
    });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.clear(screen.getByLabelText("Model ID"));
    await user.type(screen.getByLabelText("Model ID"), "new-model:high");
    await user.clear(screen.getByLabelText("Context Window"));
    await user.type(screen.getByLabelText("Context Window"), "1024");
    await user.clear(screen.getByLabelText("Max Tokens"));
    await user.type(screen.getByLabelText("Max Tokens"), "2048");

    const create = screen.getByRole("button", { name: "创建 Provider" });
    await waitFor(() => expect(create).toBeEnabled());
    await user.click(create);
    await waitFor(() => expect(createCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
      firstModel: expect.objectContaining({
        id: "new-model:high",
        contextWindow: 1_024,
        maxTokens: 2_048,
      }),
    })));
  });

  it("retries a failed Provider detail read", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn()
      .mockRejectedValueOnce({
        code: "overview-read-failed",
        message: "无法读取 Provider 配置。",
        action: "请重新读取。",
      })
      .mockResolvedValue(overviewLoad(overviewDto(), readyState));
    renderRoute("/providers/dnslin", { ...unavailableClient, getOverviewLoad });

    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取 Provider 配置。");
    await user.click(screen.getByRole("button", { name: "重新读取" }));
    expect(await screen.findByRole("heading", { name: "dnslin" })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("explains when the requested Provider no longer exists", async () => {
    renderRoute("/providers/missing-provider", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    expect(await screen.findByRole("heading", { name: "Provider 不存在" })).toBeVisible();
    expect(screen.getByRole("link", { name: "返回 Providers" })).toHaveAttribute("href", "/providers");
  });

  it("keeps the completed wizard open on a models Hash conflict until the user explicitly reloads", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto(), readyState));
    const createCustomProvider = vi.fn(async () => {
      throw {
        code: "models-hash-conflict",
        message: "models.yml 在打开表单后已被外部修改。",
        action: "请重新读取配置；当前表单输入已保留，OMP Switch 不会自动合并。",
      };
    });
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, createCustomProvider });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.click(screen.getByRole("button", { name: "创建 Provider" }));

    expect(await screen.findByText("配置冲突")).toBeVisible();
    expect(screen.getByText("models.yml 在打开表单后已被外部修改。")).toBeVisible();
    expect(screen.getByLabelText("Model ID")).toHaveValue("new-model");
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(await screen.findByLabelText("Provider ID")).toHaveValue("new-provider");
    expect(screen.getByRole("dialog")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("keeps the conflict form open when rereading configuration fails", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn()
      .mockResolvedValueOnce(overviewLoad(overviewDto(), readyState))
      .mockResolvedValueOnce({
        startupState: readyState,
        overview: null,
        error: {
          code: "overview-read-failed",
          message: "无法重新读取 models.yml。",
          action: "请检查文件后重试。",
        },
      });
    const createCustomProvider = vi.fn(async () => {
      throw {
        code: "models-hash-conflict",
        message: "models.yml 在打开表单后已被外部修改。",
        action: "请重新读取配置；当前表单输入已保留，OMP Switch 不会自动合并。",
      };
    });
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, createCustomProvider });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.click(screen.getByRole("button", { name: "创建 Provider" }));
    await user.click(await screen.findByRole("button", { name: "重新读取" }));

    const conflictDialog = screen.getByRole("dialog");
    expect(await within(conflictDialog).findByText("无法重新读取 models.yml。")).toBeVisible();
    expect(screen.getByLabelText("Model ID")).toHaveValue("new-model");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("keeps submitted model values available after a non-conflict Provider write failure", async () => {
    const user = userEvent.setup();
    const createCustomProvider = vi.fn(async () => {
      throw {
        code: "provider-create-failed",
        message: "无法替换 models.yml。",
        action: "请检查文件权限后重试。",
      };
    });
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      createCustomProvider,
    });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.click(screen.getByRole("button", { name: "创建 Provider" }));

    expect(await screen.findByText("无法创建 Provider")).toBeVisible();
    expect(screen.getByText("无法替换 models.yml。")).toBeVisible();
    expect(screen.getByLabelText("Model ID")).toHaveValue("new-model");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(createCustomProvider).toHaveBeenCalledTimes(1);
  });

  it("allows a stale API Key value when authentication is explicitly disabled", async () => {
    const user = userEvent.setup();
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
    await user.type(screen.getByLabelText("Provider ID"), "no-auth-provider");
    await user.type(screen.getByLabelText("Base URL"), "https://no-auth.example/v1");
    await user.type(screen.getByLabelText("API Key", { selector: 'input[type="password"]' }), "!stale-key");
    await user.click(screen.getByRole("radio", { name: "无需认证" }));
    await user.click(screen.getByRole("button", { name: "下一步" }));

    expect(await screen.findByText((_, element) => Boolean(
      element?.classList.contains("provider-create-step")
      && element.textContent === "步骤 2 / 2 · 首个模型",
    ))).toBeVisible();
    expect(screen.queryByText("Direct API Key 不能以 ! 开头。")).not.toBeInTheDocument();
  });

  it.each([
    ["provider-id-invalid", "provider", "Provider ID"],
    ["provider-id-conflict", "provider", "Provider ID"],
    ["provider-base-url-invalid", "provider", "Base URL"],
    ["provider-api-key-invalid", "provider", "API Key"],
    ["provider-auth-invalid", "provider", "API Key"],
    ["model-id-invalid", "model", "Model ID"],
    ["model-name-required", "model", "名称"],
    ["model-api-required", "model", "协议"],
    ["model-input-required", "model", "能力"],
    ["model-context-window-invalid", "model", "Context Window"],
    ["model-token-limit-invalid", "model", "Max Tokens"],
  ] as const)("routes server validation error %s to its visible field", async (code, expectedStep, fieldLabel) => {
    const user = userEvent.setup();
    const message = `服务器返回的 ${code} 错误。`;
    const createCustomProvider = vi.fn(async () => {
      throw { code, message, action: "请修正字段后重试。" };
    });
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      createCustomProvider,
    });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    await user.click(screen.getByRole("button", { name: "创建 Provider" }));

    const error = await screen.findByText(message);
    const stepText = expectedStep === "provider" ? "步骤 1 / 2 · Provider" : "步骤 2 / 2 · 首个模型";
    expect(screen.getByText((_, element) => Boolean(
      element?.classList.contains("provider-create-step") && element.textContent === stepText,
    ))).toBeVisible();
    if (fieldLabel === "能力") {
      expect(error.closest("fieldset")).toHaveTextContent(fieldLabel);
    } else {
      expect(error.closest(".provider-create-field")).toHaveTextContent(fieldLabel);
    }
  });

  it("confirms dirty wizard dismissal and closes a clean wizard immediately", async () => {
    const user = userEvent.setup();
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
    await user.type(screen.getByLabelText("Provider ID"), "draft-provider");
    await user.click(screen.getByRole("button", { name: "取消" }));
    const cancelHeading = await screen.findByRole("heading", { name: "有未保存的修改" });
    const cancelDialog = cancelHeading.closest('[role="dialog"]');
    expect(cancelDialog).not.toBeNull();
    await user.click(within(cancelDialog as HTMLElement).getByRole("button", { name: "继续编辑" }));
    expect(screen.getByLabelText("Provider ID")).toHaveValue("draft-provider");

    await user.click(screen.getByRole("button", { name: "取消" }));
    const discardHeading = await screen.findByRole("heading", { name: "有未保存的修改" });
    const discardDialog = discardHeading.closest('[role="dialog"]');
    expect(discardDialog).not.toBeNull();
    await user.click(within(discardDialog as HTMLElement).getByRole("button", { name: "放弃修改" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
    await user.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("confirms before navigating away from a dirty Provider wizard", async () => {
    const user = userEvent.setup();
    const { router } = renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
    await user.type(screen.getByLabelText("Provider ID"), "draft-provider");
    void router.navigate("/overview");

    const firstConfirmation = await screen.findByRole("heading", { name: "有未保存的修改" });
    const firstDialog = firstConfirmation.closest('[role="dialog"]');
    expect(firstDialog).not.toBeNull();
    await user.click(within(firstDialog as HTMLElement).getByRole("button", { name: "继续编辑" }));
    expect(screen.getByLabelText("Provider ID")).toHaveValue("draft-provider");

    void router.navigate("/overview");
    const secondConfirmation = await screen.findByRole("heading", { name: "有未保存的修改" });
    const secondDialog = secondConfirmation.closest('[role="dialog"]');
    expect(secondDialog).not.toBeNull();
    await user.click(within(secondDialog as HTMLElement).getByRole("button", { name: "放弃修改" }));

    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
  });

  it("latches rapid Provider creation submissions", async () => {
    const user = userEvent.setup();
    const createCustomProvider = vi.fn(() => new Promise<{ providerId: string; modelId: string }>(() => undefined));
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      createCustomProvider,
    });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1",
      modelId: "new-model",
      modelName: "New Model",
    });
    const create = screen.getByRole("button", { name: "创建 Provider" });
    create.click();
    create.click();

    await waitFor(() => expect(createCustomProvider).toHaveBeenCalledTimes(1));
  });

  it("matches the masked-key glyph and model protocol source treatment", async () => {
    const user = userEvent.setup();
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    await user.click(await screen.findByRole("button", { name: "新增 Provider" }));
    expect(screen.getByRole("button", { name: "显示 API Key" }).querySelector("svg")).toHaveClass("lucide-eye-off");
    await user.type(screen.getByLabelText("Provider ID"), "new-provider");
    await user.type(screen.getByLabelText("Base URL"), "https://new-provider.example/v1");
    await user.click(screen.getByRole("button", { name: "下一步" }));

    const protocol = await screen.findByRole("combobox", { name: "协议" });
    expect(protocol).toHaveTextContent("openai-responses");
    expect(protocol).toHaveTextContent("模型指定");
    expect(protocol.querySelector(".lucide-chevron-down")).not.toBeInTheDocument();
  });

  it("uses URL-safe endpoint construction in the Provider wizard", async () => {
    const user = userEvent.setup();
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
    });

    await fillProviderWizard(user, {
      providerId: "new-provider",
      baseUrl: "https://new-provider.example/v1?region=us",
      modelId: "new-model",
      modelName: "New Model",
    });

    expect(screen.getByLabelText("最终地址")).toHaveValue("https://new-provider.example/v1/responses?region=us");
  });
 
  it("lists searchable Provider safety summaries while allowing safe detail viewing", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const openAiModel: OverviewModel = {
      ...base.models[0],
      providerId: "OpenAI",
      id: "GPT-5.6-SOL",
      name: "GPT-5.6 Sol",
    };
    const advancedModel: OverviewModel = {
      ...base.models[0],
      providerId: "advanced",
      id: "claude-opus",
      name: "Claude Opus",
      effectiveApi: "anthropic-messages",
    };
    const providers: OverviewProvider[] = [
      {
        ...base.providers[0],
        id: "OpenAI",
        name: "OpenAI",
        baseUrl: "https://api.openai.com/v1",
        modelCount: 1,
        classification: "built-in-override",
        editable: false,
        readOnlyReason: "Provider 或 Model ID 覆盖 OMP bundled catalog，只能查看。",
        models: [openAiModel],
      },
      {
        ...base.providers[0],
        id: "advanced",
        name: "Advanced endpoint",
        baseUrl: "https://advanced.example/v1",
        authMode: "unsupported",
        hasApiKey: false,
        defaultApi: "anthropic-messages",
        modelCount: 1,
        classification: "advanced",
        editable: false,
        readOnlyReason: "包含 OMP Switch 不支持的高级配置。",
        models: [advancedModel],
      },
    ];
    const lockedOverview = overviewDto({
      state: "read-only",
      counts: { providerCount: 0, modelCount: 2, roleCount: 2 },
      providers,
      models: [openAiModel, advancedModel],
      readOnlyReason: "当前配置包含以下只读 Provider 分类：OMP 内置 Provider/Model 覆盖、高级 Provider。",
    });
    const getOverviewLoad = vi.fn(async () => overviewLoad(lockedOverview, readyState));
    const page = renderRoute("/providers", { ...unavailableClient, getOverviewLoad });

    expect(await screen.findByRole("heading", { name: "Providers" })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("columnheader", { name: "Provider ID" })).toBeVisible();
    expect(screen.getByRole("columnheader", { name: "Base URL" })).toBeVisible();
    expect(screen.getByText("API Key 已配置")).toBeVisible();
    expect(screen.getByText("包含 OMP Switch 不支持的高级配置。")).toBeVisible();
    expect(screen.getByText("内置覆盖 · 只读")).toBeVisible();
    expect(screen.getByText("高级配置 · 只读")).toBeVisible();
    expect(screen.getByRole("button", { name: "新增 Provider" })).toBeDisabled();
    expect(screen.getByRole("link", { name: "OpenAI 详情" })).toHaveAttribute("href", "/providers/OpenAI");
    expect(screen.getByRole("link", { name: "advanced 详情" })).toHaveAttribute("href", "/providers/advanced");

    await user.type(screen.getByRole("searchbox", { name: "搜索 Provider" }), "claude");
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
    expect(screen.getByText("advanced")).toBeVisible();

    page.unmount();
    const unavailableCatalog = overviewDto({
      state: "read-only",
      providers: [{
        ...base.providers[0],
        id: "custom",
        modelCount: 1,
        classification: "unavailable",
        editable: false,
        readOnlyReason: "当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。",
      }],
      readOnlyReason: "当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。",
    });
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(unavailableCatalog, readyState),
    });

    expect(await screen.findAllByText("当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。")).toHaveLength(2);
    expect(screen.getByText("清单缺失 · 只读")).toBeVisible();
    expect(screen.getByRole("button", { name: "新增 Provider" })).toBeDisabled();
  });
  it("marks an otherwise Custom Provider with an incomplete model", async () => {
    const base = overviewDto();
    const incompleteModel: OverviewModel = {
      ...base.models[0],
      complete: false,
      editable: false,
      readOnlyReason: "Model definition 配置不完整。",
    };
    const incompleteProvider: OverviewProvider = {
      ...base.providers[0],
      editable: true,
      classification: "custom",
      models: [incompleteModel],
    };
    const overview = overviewDto({
      providers: [incompleteProvider],
      models: [incompleteModel],
    });

    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overview, readyState),
    });

    expect(await screen.findByText("配置不完整")).toBeVisible();
  });
  it("does not run a second startup detection while Overview owns loading", async () => {
    const getStartupState = vi.fn(unavailableClient.getStartupState);
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto()));
    renderRoute("/overview", { ...unavailableClient, getStartupState, getOverviewLoad });

    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(1);
    expect(getStartupState).not.toHaveBeenCalled();
  });
  it("keeps the connected OMP status footer after navigating away from Overview", async () => {
    const user = userEvent.setup();
    renderRoute("/overview", {
      ...unavailableClient,
      getStartupState: async () => readyState,
      getOverviewLoad: async () => overviewLoad(overviewDto()),
    });

    await user.click(await screen.findByRole("link", { name: "Providers" }));
    expect(await screen.findByRole("heading", { name: "Providers" })).toBeVisible();
    expect(screen.getByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible();
  });
  it("refreshes the sidebar status when navigating away from Providers", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto(), readyState));
    const getStartupState = vi.fn(async () => ({ kind: "omp-unavailable", message: "OMP 已不可用" } as const));
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, getStartupState });

    expect(await screen.findByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible();
    await user.click(screen.getByRole("link", { name: "角色" }));
    expect(await screen.findByRole("heading", { name: "角色" })).toBeVisible();
    expect(await screen.findByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
    expect(getStartupState).not.toHaveBeenCalled();
  });
  it("shows Providers load errors without exposing stale startup context", async () => {
    renderRoute("/providers", {
      ...unavailableClient,
      getOverviewLoad: async () => {
        throw { code: "omp-path-failed", message: "无法检查 OMP PATH", action: "请重新检测 OMP。" };
      },
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("无法检查 OMP PATH");
    expect(screen.getByRole("link", { name: /OMP 状态不可用.*请重新读取 OMP/ })).toBeVisible();
  });
  it("clears the stale OMP footer after a Providers retry fails", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn()
      .mockResolvedValueOnce({
        startupState: readyState,
        overview: null,
        error: { code: "overview-read-failed", message: "首次读取失败", action: "请重新读取。" },
      })
      .mockRejectedValueOnce({ code: "omp-path-failed", message: "重试读取失败", action: "请重新检测 OMP。" });
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad });

    expect(await screen.findByRole("alert")).toHaveTextContent("首次读取失败");

    await user.click(screen.getByRole("button", { name: "重新读取" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("重试读取失败");
    expect(screen.getByRole("link", { name: /OMP 状态不可用.*请重新读取 OMP/ })).toBeVisible();
  });
  it("ignores stale Providers data after navigation", async () => {
    const user = userEvent.setup();
    const first = deferred<OverviewLoad>();
    const getOverviewLoad = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce(overviewLoad(overviewDto(), readyState));
    const getStartupState = vi.fn(async () => ({ kind: "omp-unavailable", message: "最新 OMP 状态不可用" } as const));
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, getStartupState });

    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("link", { name: "角色" }));
    expect(await screen.findByRole("heading", { name: "角色" })).toBeVisible();
    expect(await screen.findByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible();
    first.resolve(overviewLoad(overviewDto(), readyState));
    await waitFor(() => expect(screen.getByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible());
  });

  it("renders navigation and page content as sibling shell regions", () => {
    renderRoute("/overview");

    const shell = screen.getByRole("main");
    const sidebar = screen.getByRole("navigation", { name: "主导航" }).closest("aside");
    const content = screen.getByRole("heading", { name: "概览" }).closest("section");

    expect(sidebar?.parentElement).toBe(shell);
    expect(content?.parentElement).toBe(shell);
    expect(Array.from(shell.children)).toEqual([sidebar, content]);
  });

  it("falls back safely for an unknown route", () => {
    renderRoute("/not-a-real-page");

    expect(screen.getByRole("heading", { name: "页面不存在" })).toBeVisible();
    expect(screen.getByRole("link", { name: "返回概览" })).toHaveAttribute("href", "/overview");
  });

  it("shows settings read errors while keeping Overview selection session-only", async () => {
    const user = userEvent.setup();
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => {
        throw {
          code: "settings-read-failed",
          message: "无法读取界面状态",
          action: "请检查应用数据目录权限。",
          internal: "sensitive internal detail",
        };
      },
      saveUiSettings,
    });

    expect(await screen.findByText("无法读取界面状态")).toBeVisible();
    expect(screen.getByText("请检查应用数据目录权限。")).toBeVisible();
    expect(screen.queryByText("sensitive internal detail")).not.toBeInTheDocument();
    const provider = screen.getByRole("combobox", { name: "Provider" });
    const model = screen.getByRole("combobox", { name: "模型" });
    expect(provider).toHaveTextContent("dnslin");
    expect(model).toHaveTextContent("gpt-5.6-sol");

    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "anthropic" }));
    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "claude-sonnet-4" }));
    expect(model).toHaveTextContent("claude-sonnet-4");
    expect(saveUiSettings).not.toHaveBeenCalled();
  });
  it("submits only an explicit keep-key intent from the Provider editor", async () => {
    const user = userEvent.setup();
    const storedKey = "fixture-stored-direct-key-must-not-render";
    const editCustomProvider = vi.fn(async () => ({ providerId: "dnslin" }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    const dialog = screen.getByRole("dialog");
    const id = within(dialog).getByLabelText("Provider ID");
    const apiKey = within(dialog).getByLabelText("API Key", { selector: 'input[type="password"]' });
    expect(id).toHaveValue("dnslin");
    expect(id).toHaveAttribute("readonly");
    expect(apiKey).toHaveValue("");
    expect(dialog).not.toHaveTextContent(storedKey);

    await user.clear(within(dialog).getByLabelText("Base URL"));
    await user.type(within(dialog).getByLabelText("Base URL"), "https://edited.example/v1");
    await user.type(apiKey, "   ");
    await user.click(within(dialog).getByRole("button", { name: "保存 Provider" }));

    await waitFor(() => expect(editCustomProvider).toHaveBeenCalledWith({
      openedModelsHash: "models-hash",
      providerId: "dnslin",
      baseUrl: "https://edited.example/v1",
      defaultApi: "openai-responses",
      authMode: "api-key",
      apiKey: { kind: "keep" },
    }));
  });
  it("keeps the Provider editor concurrency hash across a background refresh", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const refreshed = overviewDto({
      files: {
        ...base.files,
        models: { ...base.files.models, contentHash: "models-hash-after-refresh" },
      },
    });
    let loadCount = 0;
    const getOverviewLoad = vi.fn(async () => overviewLoad(loadCount++ === 0 ? base : refreshed, readyState));
    const editCustomProvider = vi.fn(async () => ({ providerId: "dnslin" }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad,
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    act(() => {
      useModelTestStore.getState().finish({
        success: true,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses",
        latencyMs: 1,
        status: 200,
        message: "模型连接成功",
      });
    });
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));

    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Base URL"));
    await user.type(within(dialog).getByLabelText("Base URL"), "https://edited.example/v1");
    await user.click(within(dialog).getByRole("button", { name: "保存 Provider" }));

    await waitFor(() => expect(editCustomProvider).toHaveBeenCalledWith(expect.objectContaining({ openedModelsHash: "models-hash" })));
  });

  it("keeps an unchanged no-auth credential while saving other Provider fields", async () => {
    const user = userEvent.setup();
    const editCustomProvider = vi.fn(async () => ({ providerId: "dnslin" }));
    const noAuthProvider: OverviewProvider = {
      ...overviewDto().providers[0],
      authMode: "none",
      hasApiKey: false,
    };
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [noAuthProvider] }), readyState),
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Base URL"));
    await user.type(within(dialog).getByLabelText("Base URL"), "https://edited.example/v1");
    await user.click(within(dialog).getByRole("button", { name: "保存 Provider" }));

    await waitFor(() => expect(editCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
      authMode: "none",
      apiKey: { kind: "keep" },
    })));
  });

  it("confirms Direct API Key deletion before switching to no authentication", async () => {
    const user = userEvent.setup();
    const typedKey = "fixture-typed-direct-key-must-not-escape";
    const editCustomProvider = vi.fn(async () => ({ providerId: "dnslin" }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    const editor = screen.getByRole("dialog");
    const keyInput = within(editor).getByLabelText("API Key", { selector: 'input[type="password"]' });
    await user.type(keyInput, typedKey);
    await user.click(within(editor).getByRole("radio", { name: "无需认证" }));

    const confirmationHeading = await screen.findByRole("heading", { name: "删除 Direct API Key？" });
    const confirmation = confirmationHeading.closest('[role="dialog"]') as HTMLElement;
    expect(confirmation).toHaveTextContent("删除当前保存的 Direct API Key");
    await user.click(within(confirmation).getByRole("button", { name: "继续编辑" }));
    expect(within(editor).getByRole("radio", { name: "API Key 认证" })).toBeChecked();
    expect(keyInput).not.toHaveValue("");

    await user.click(within(editor).getByRole("radio", { name: "无需认证" }));
    const secondHeading = await screen.findByRole("heading", { name: "删除 Direct API Key？" });
    const secondConfirmation = secondHeading.closest('[role="dialog"]') as HTMLElement;
    await user.click(within(secondConfirmation).getByRole("button", { name: "删除并切换为无需认证" }));
    expect(within(editor).getByRole("radio", { name: "无需认证" })).toBeChecked();
    expect(within(editor).queryByLabelText("API Key", { selector: 'input[type="password"]' })).not.toBeInTheDocument();

    await user.click(within(editor).getByRole("button", { name: "保存 Provider" }));
    await waitFor(() => expect(editCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
      authMode: "none",
      apiKey: { kind: "delete" },
    })));
    expect(JSON.stringify(editCustomProvider.mock.calls)).not.toContain(typedKey);
  });

  it("retains the Provider draft through a conflict and confirms reload discard", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto(), readyState));
    const editCustomProvider = vi.fn(async () => {
      throw {
        code: "models-hash-conflict",
        message: "models.yml 在打开表单后已被外部修改。",
        action: "请重新读取配置；当前表单输入已保留，OMP Switch 不会自动合并。",
      };
    });
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad,
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    const editor = screen.getByRole("dialog");
    const baseUrl = within(editor).getByLabelText("Base URL");
    await user.clear(baseUrl);
    await user.type(baseUrl, "https://conflict.example/v1");
    await user.click(within(editor).getByRole("button", { name: "保存 Provider" }));
    expect(await within(editor).findByText("配置冲突")).toBeVisible();
    expect(baseUrl).toHaveValue("https://conflict.example/v1");

    await user.click(within(editor).getByRole("button", { name: "重新读取" }));
    const confirmationHeading = await screen.findByRole("heading", { name: "重新读取 Provider？" });
    const confirmation = confirmationHeading.closest('[role="dialog"]') as HTMLElement;
    expect(confirmation).toHaveTextContent("丢失当前未保存的修改");
    await user.click(within(confirmation).getByRole("button", { name: "继续编辑" }));
    expect(baseUrl).toHaveValue("https://conflict.example/v1");

    await user.click(within(editor).getByRole("button", { name: "重新读取" }));
    const secondHeading = await screen.findByRole("heading", { name: "重新读取 Provider？" });
    const secondConfirmation = secondHeading.closest('[role="dialog"]') as HTMLElement;
    await user.click(within(secondConfirmation).getByRole("button", { name: "重新读取并丢弃修改" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("shows a detailed write failure in the editor without exposing a Direct API Key", async () => {
    const user = userEvent.setup();
    const typedKey = "fixture-write-failure-direct-key-must-not-escape";
    const editCustomProvider = vi.fn(async () => {
      throw {
        code: "provider-edit-failed",
        message: "无法安全写入 models.yml。",
        action: "请检查路径、权限和可用磁盘空间后重试。",
        internal: typedKey,
      };
    });
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      editCustomProvider,
    });

    await user.click(await screen.findByRole("button", { name: "编辑 Provider" }));
    const editor = screen.getByRole("dialog");
    const keyInput = within(editor).getByLabelText("API Key", { selector: 'input[type="password"]' });
    await user.type(keyInput, typedKey);
    await user.click(within(editor).getByRole("button", { name: "保存 Provider" }));

    expect(await within(editor).findByText("无法安全写入 models.yml。")).toBeVisible();
    expect(within(editor).getByText("请检查路径、权限和可用磁盘空间后重试。")).toBeVisible();
    expect(keyInput).not.toHaveValue("");
    expect(document.body.textContent).not.toContain(typedKey);
    expect(screen.getAllByText("无法保存 Provider").length).toBeGreaterThanOrEqual(1);
  });
  it("renders searchable Model definitions and submits the create Sheet", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const createModel = vi.fn(async () => ({ providerId: "dnslin", modelId: "new-model" }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(base, readyState),
      createModel,
    });

    expect(await screen.findByText("Sol")).toBeVisible();
    expect(screen.getByTitle("模型指定")).toBeVisible();
    await user.type(screen.getByLabelText("搜索 Model ID"), "missing-model");
    expect(screen.getByText("没有匹配的 Model definition")).toBeVisible();
    await user.clear(screen.getByLabelText("搜索 Model ID"));
    await user.click(screen.getByRole("button", { name: "新增模型" }));
    const sheet = await screen.findByRole("dialog");
    expect(within(sheet).getByRole("heading", { name: "新增模型" })).toBeVisible();
    expect(within(sheet).getByLabelText("Model ID")).toBeVisible();
    await user.type(within(sheet).getByLabelText("Model ID"), "new-model");
    await user.type(within(sheet).getByLabelText("名称"), "New Model");
    await user.click(within(sheet).getByRole("button", { name: "保存模型" }));

    await waitFor(() => expect(createModel).toHaveBeenCalledWith(expect.objectContaining({ providerId: "dnslin", model: expect.objectContaining({ id: "new-model", name: "New Model", input: ["text", "image"] }) })));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("keeps incomplete models repairable, locks read-only models, and confirms deletion", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const incomplete: OverviewModel = { ...base.models[0], id: "incomplete", name: null, contextWindow: null, maxTokens: null, input: [], complete: false, status: "incomplete", editable: true, referenceCount: 0, referencePaths: [], roleReferencePaths: [], otherReferencePaths: [], readOnlyReason: null };
    const locked: OverviewModel = { ...base.models[0], id: "locked", name: "Locked", complete: false, status: "read-only", editable: false, referenceCount: 1, referencePaths: ['config.yml:modelRoles["default"]'], roleReferencePaths: ['config.yml:modelRoles["default"]'], otherReferencePaths: [], readOnlyReason: "Model definition 包含当前版本不支持的配置，只能查看。" };
    const referenced = { ...base.models[0], referenceCount: 1, referencePaths: ['config.yml:modelRoles["default"]'], roleReferencePaths: ['config.yml:modelRoles["default"]'], otherReferencePaths: [] };
    const overview = overviewDto({
      providers: [{ ...base.providers[0], modelCount: 3, models: [referenced, incomplete, locked] }],
      models: [referenced, incomplete, locked],
      counts: { providerCount: 1, modelCount: 3, roleCount: 2 },
    });
    const createModel = vi.fn(async () => ({ providerId: "dnslin", modelId: "copied-model" }));
    const editModel = vi.fn(async () => ({ providerId: "dnslin", modelId: "incomplete" }));
    const openTargetConfigurationDirectory = vi.fn(async () => undefined);
    const deleteModel = vi.fn(async () => {
      throw { code: "model-delete-referenced", message: "无法删除 Model：仍有配置引用。", action: "请先处理引用。" };
    });
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overview, readyState),
      createModel,
      editModel,
      openTargetConfigurationDirectory,
      deleteModel,
    });

    expect(await screen.findByText("配置不完整")).toBeVisible();
    expect(screen.getByRole("button", { name: "Model 操作 locked" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Model 操作 locked" }));
    await user.click(screen.getByRole("menuitem", { name: "查看" }));
    const readOnlySheet = await screen.findByRole("dialog");
    expect(within(readOnlySheet).getByRole("heading", { name: "查看模型" })).toBeVisible();
    expect(readOnlySheet).toHaveTextContent(locked.readOnlyReason!);
    expect(within(readOnlySheet).queryByRole("button", { name: "保存模型" })).not.toBeInTheDocument();
    await user.click(within(readOnlySheet).getByRole("button", { name: "关闭" }));
    await user.click(screen.getByRole("button", { name: "Model 操作 incomplete" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const repairSheet = await screen.findByRole("dialog");
    expect(within(repairSheet).getByLabelText("Context Window")).toHaveValue(null);
    expect(within(repairSheet).getByLabelText("Max Tokens")).toHaveValue(null);
    await user.type(within(repairSheet).getByLabelText("名称"), "Repaired");
    await user.click(within(repairSheet).getByRole("checkbox", { name: "Text" }));
    await user.type(within(repairSheet).getByLabelText("Context Window"), "100000");
    await user.type(within(repairSheet).getByLabelText("Max Tokens"), "1000");
    expect(within(repairSheet).getByLabelText("Model ID")).toHaveAttribute("readonly");
    await user.click(within(repairSheet).getByRole("button", { name: "保存模型" }));
    await waitFor(() => expect(editModel).toHaveBeenCalledWith(expect.objectContaining({ modelId: "incomplete", model: expect.objectContaining({ name: "Repaired", input: ["text"], contextWindow: 100000, maxTokens: 1000 }) })));
    await user.click(screen.getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "复制" }));
    const copySheet = await screen.findByRole("dialog");
    expect(within(copySheet).getByLabelText("Model ID")).toHaveValue("gpt-5.6-sol-copy");
    expect(within(copySheet).getByLabelText("名称")).toHaveValue("Sol");
    await user.clear(within(copySheet).getByLabelText("Model ID"));
    await user.type(within(copySheet).getByLabelText("Model ID"), "copied-model");
    await user.click(within(copySheet).getByRole("button", { name: "保存模型" }));
    await waitFor(() => expect(createModel).toHaveBeenCalledWith(expect.objectContaining({ model: expect.objectContaining({ id: "copied-model", name: "Sol" }) })));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());


    await user.click(screen.getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const confirmation = await screen.findByRole("heading", { name: "删除模型？" });
    const dialog = confirmation.closest('[role="dialog"]') as HTMLElement;
    expect(dialog).toHaveTextContent("当前不会部分删除；需要 Configuration transaction 同时更新 models.yml 和 config.yml。");
    expect(dialog).toHaveTextContent('config.yml:modelRoles["default"]');
    expect(within(dialog).getByRole("button", { name: "删除模型" })).toBeDisabled();
    expect(dialog).toHaveTextContent("不会写入配置，也不会创建备份。");
    await user.click(within(dialog).getByRole("button", { name: "打开配置目录" }));
    expect(openTargetConfigurationDirectory).toHaveBeenCalledWith("/usr/local/bin/omp");
    expect(deleteModel).not.toHaveBeenCalled();
  });
  it("shows a complete model deletion impact and refreshes after a safe delete", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const second: OverviewModel = { ...base.models[0], id: "second", name: "Second", referenceCount: 0, referencePaths: [], roleReferencePaths: [], otherReferencePaths: [] };
    const provider: OverviewProvider = { ...base.providers[0], modelCount: 2, models: [base.models[0], second] };
    const initial = overviewDto({ providers: [provider], models: [base.models[0], second], counts: { providerCount: 1, modelCount: 2, roleCount: 0 }, roles: [] });
    const remainingProvider: OverviewProvider = { ...base.providers[0], modelCount: 1, models: [base.models[0]] };
    const after = overviewDto({ providers: [remainingProvider], models: [base.models[0]], counts: { providerCount: 1, modelCount: 1, roleCount: 0 }, roles: [] });
    const getOverviewLoad = vi.fn().mockResolvedValueOnce(overviewLoad(initial, readyState)).mockResolvedValueOnce(overviewLoad(after, readyState));
    const deleteModel = vi.fn().mockResolvedValue({ providerId: "dnslin", modelId: "second" });
    renderRoute("/providers/dnslin", { ...unavailableClient, getOverviewLoad, deleteModel });

    await screen.findByText("Second");
    await user.click(screen.getByRole("button", { name: "Model 操作 second" }));
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("dnslin/second");
    expect(dialog).toHaveTextContent("受影响 Model role");
    expect(dialog).toHaveTextContent("无");
    expect(dialog).toHaveTextContent("此操作会创建备份");
    await user.click(within(dialog).getByRole("button", { name: "删除模型" }));
    await waitFor(() => expect(deleteModel).toHaveBeenCalledWith({ openedModelsHash: "models-hash", openedConfigHash: "config-hash", providerId: "dnslin", modelId: "second" }));
    await waitFor(() => expect(screen.queryByText("Second")).not.toBeInTheDocument());
  });

  it("shows Provider deletion impact and blocks unmanaged references", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const referenced = { ...base.models[0], referenceCount: 1, referencePaths: ['config.yml:retry["fallback"]'], roleReferencePaths: [], otherReferencePaths: ['config.yml:retry["fallback"]'] };
    const provider: OverviewProvider = { ...base.providers[0], modelCount: 1, roleReferencePaths: [], otherReferencePaths: ['config.yml:retry["fallback"]'], models: [referenced] };
    const overview = overviewDto({ providers: [provider], models: [referenced], roles: [], counts: { providerCount: 1, modelCount: 1, roleCount: 0 } });
    const openTargetConfigurationDirectory = vi.fn(async () => undefined);
    const deleteProvider = vi.fn().mockResolvedValue({ providerId: "dnslin", modelCount: 1 });
    renderRoute("/providers/dnslin", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview, readyState), openTargetConfigurationDirectory, deleteProvider });

    await screen.findByText("Sol");
    await user.click(screen.getByRole("button", { name: "删除 Provider" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("包含模型");
    expect(dialog).toHaveTextContent("gpt-5.6-sol");
    expect(dialog).toHaveTextContent("config.yml:retry[\"fallback\"]");
    expect(dialog).toHaveTextContent("不会修改");
    expect(dialog).toHaveTextContent("不会写入配置，也不会创建备份。");
    await user.click(within(dialog).getByRole("button", { name: "打开配置目录" }));
    expect(openTargetConfigurationDirectory).toHaveBeenCalledWith("/usr/local/bin/omp");
    expect(within(dialog).getByRole("button", { name: "删除 Provider" })).toBeDisabled();
    expect(deleteProvider).not.toHaveBeenCalled();
  });
  it("blocks Provider deletion when an included Model is read-only", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const readOnlyModel: OverviewModel = {
      ...base.models[0],
      id: "advanced-model",
      name: "Advanced model",
      editable: false,
      status: "read-only",
      readOnlyReason: "Model definition 包含当前版本不支持的配置，只能查看。",
      referenceCount: 0,
      referencePaths: [],
      roleReferencePaths: [],
      otherReferencePaths: [],
    };
    const provider: OverviewProvider = {
      ...base.providers[0],
      modelCount: 1,
      models: [readOnlyModel],
      roleReferencePaths: [],
      otherReferencePaths: [],
    };
    const overview = overviewDto({ providers: [provider], models: [readOnlyModel], roles: [], counts: { providerCount: 1, modelCount: 1, roleCount: 0 } });
    const deleteProvider = vi.fn().mockResolvedValue({ providerId: "dnslin", modelCount: 1 });
    renderRoute("/providers/dnslin", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview, readyState), deleteProvider });

    await screen.findByText("Advanced model");
    await user.click(screen.getByRole("button", { name: "删除 Provider" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Provider 包含只读 Model definition advanced-model");
    expect(within(dialog).getByRole("button", { name: "删除 Provider" })).toBeDisabled();
    expect(deleteProvider).not.toHaveBeenCalled();
  });

  it("deletes an unreferenced Provider and returns to the Provider list", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const second = { ...base.models[0], id: "second", name: "Second", referenceCount: 0, referencePaths: [], roleReferencePaths: [], otherReferencePaths: [] };
    const provider: OverviewProvider = { ...base.providers[0], modelCount: 2, roleReferencePaths: [], otherReferencePaths: [], models: [base.models[0], second] };
    const initial = overviewDto({ providers: [provider], models: [base.models[0], second], counts: { providerCount: 1, modelCount: 2, roleCount: 0 }, roles: [] });
    const after = overviewDto({ state: "empty", providers: [], models: [], counts: { providerCount: 0, modelCount: 0, roleCount: 0 }, roles: [] });
    const getOverviewLoad = vi.fn().mockResolvedValueOnce(overviewLoad(initial, readyState)).mockResolvedValue(overviewLoad(after, readyState));
    const deleteProvider = vi.fn().mockResolvedValue({ providerId: "dnslin", modelCount: 2 });
    renderRoute("/providers/dnslin", { ...unavailableClient, getOverviewLoad, deleteProvider });

    await screen.findByText("Sol");
    await user.click(screen.getByRole("button", { name: "删除 Provider" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("dnslin");
    expect(dialog).toHaveTextContent("gpt-5.6-sol");
    expect(dialog).toHaveTextContent("second");
    expect(dialog).toHaveTextContent("此操作会创建备份");
    await user.click(within(dialog).getByRole("button", { name: "删除 Provider" }));

    await waitFor(() => expect(deleteProvider).toHaveBeenCalledWith({ openedModelsHash: "models-hash", openedConfigHash: "config-hash", providerId: "dnslin" }));
    expect(await screen.findByText("尚未配置 Provider。")).toBeVisible();
  });

  it("renders the ten built-in roles with the approved page skeleton", async () => {
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    renderRoute("/roles", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto()),
      saveModelRoles,
    } as TauriClient);

    expect(await screen.findByRole("heading", { name: "角色" })).toBeVisible();
    for (const roleId of ["default", "smol", "slow", "vision", "plan", "designer", "commit", "tiny", "task", "advisor"]) {
      expect(screen.getByText(roleId, { exact: true })).toBeVisible();
    }
    expect(screen.getByRole("textbox", { name: "搜索角色" })).toBeVisible();
    expect(screen.getByRole("button", { name: "保存修改" })).toBeDisabled();
    expect(saveModelRoles).not.toHaveBeenCalled();
  });

  it("sets supported thinking level and clears a built-in role", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()), saveModelRoles } as TauriClient);

    const defaultRow = await screen.findByText("default", { exact: true });
    const row = defaultRow.closest("tr") as HTMLElement;
    const thinking = within(row).getByRole("combobox", { name: "Thinking default" });
    await waitFor(() => expect(thinking).toHaveTextContent("max"));
    await user.click(thinking);
    await user.click(await screen.findByRole("option", { name: "high" }));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Thinking default" })).toHaveTextContent("high"));

    await user.click(within(row).getByRole("button", { name: "清除" }));
    expect(within(row).getByRole("combobox", { name: "Provider default" })).toHaveTextContent("未配置");
    fireEvent.keyDown(window, { key: "f", metaKey: true });
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "搜索角色" }));
    fireEvent.keyDown(window, { key: "s", metaKey: true });
    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith({
      openedConfigHash: "config-hash",
      changes: expect.arrayContaining([{ kind: "clear", roleId: "default" }]),
    }));
  });
  it("confirms clearing all built-in roles", async () => {
    const user = userEvent.setup();
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()) } as TauriClient);
    await screen.findByText("default", { exact: true });
    await user.click(screen.getByRole("button", { name: "更多角色操作" }));
    const confirmation = await screen.findByRole("heading", { name: "清除全部内置角色？" });
    await user.click(within(confirmation.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "清除" }));
    const row = screen.getByText("default", { exact: true }).closest("tr") as HTMLElement;
    expect(within(row).getByRole("combobox", { name: "Provider default" })).toHaveTextContent("未配置");
    expect(screen.getByRole("button", { name: "保存修改" })).toBeEnabled();
  });

  it("persists deletion of an existing custom role", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    const overview = overviewDto({ roles: [{ id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    await screen.findByText("researcher", { exact: true });
    await user.click(screen.getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const confirmation = await screen.findByRole("heading", { name: "删除自定义角色？" });
    await user.click(within(confirmation.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "删除角色" }));
    expect(screen.getByRole("button", { name: "保存修改" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "保存修改" }));
    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith(expect.objectContaining({ changes: [{ kind: "delete", roleId: "researcher" }] })));
  });

  it("shows unsupported protocol references without locking the role page", async () => {
    const base = overviewDto();
    const model: OverviewModel = { ...base.models[0], status: "read-only", editable: false, readOnlyReason: "Model definition 使用了不支持的协议。" };
    const normalModel: OverviewModel = { ...base.models[0], id: "gpt-5.6-normal" };
    const overview = overviewDto({
      counts: { providerCount: 1, modelCount: 2, roleCount: 1 },
      models: [model, normalModel],
      providers: [{ ...base.providers[0], modelCount: 2, models: [model, normalModel] }],
      roles: [{ id: "default", status: "unsupported", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }],
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    await screen.findByText("不支持协议");
    expect(screen.getByRole("button", { name: "新增自定义角色" })).toBeEnabled();
    expect(screen.queryByText("以下角色使用当前版本不支持的高级选择器")).not.toBeInTheDocument();
  });

  it("keeps role clearing and deletion available when assignment catalog is missing", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    const overview = overviewDto({ state: "read-only", rolesEditable: true, rolesAssignable: false, rolesReadOnlyReason: null, roles: [{ id: "default", status: "configured", selector: "dnslin/gpt-5.6-sol:max", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: "max" }, { id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    const defaultRow = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    expect(screen.getByRole("button", { name: "新增自定义角色" })).toBeDisabled();
    expect(within(defaultRow).getByRole("button", { name: "清除" })).toBeEnabled();
    await user.click(within(defaultRow).getByRole("button", { name: "清除" }));
    const customRow = screen.getByText("researcher", { exact: true }).closest("tr") as HTMLElement;
    await user.click(within(customRow).getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const confirmation = await screen.findByRole("heading", { name: "删除自定义角色？" });
    await user.click(within(confirmation.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "删除角色" }));
    await user.click(screen.getByRole("button", { name: "保存修改" }));
    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith(expect.objectContaining({ changes: expect.arrayContaining([{ kind: "clear", roleId: "default" }, { kind: "delete", roleId: "researcher" }]) })));
  });

  it("confirms before discarding a dirty role editor", async () => {
    const user = userEvent.setup();
    const { router } = renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()) } as TauriClient);
    await screen.findByText("default", { exact: true });
    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const editor = await screen.findByRole("dialog");
    const roleInput = within(editor).getByRole("textbox", { name: "角色名称" });
    await user.type(roleInput, "draft");
    router.navigate("/providers");
    const navigationBlock = await screen.findByRole("heading", { name: "有未保存的修改" });
    await user.click(within(navigationBlock.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "取消" }));
    expect(router.state.location.pathname).toBe("/roles");
    fireEvent.keyDown(editor, { key: "Escape" });
    const discard = await screen.findByRole("heading", { name: "有未保存的修改" });
    await user.click(within(discard.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "继续编辑" }));
    expect(roleInput).toHaveValue("draft");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "取消" }));
    const secondDiscard = await screen.findByRole("heading", { name: "有未保存的修改" });
    await user.click(within(secondDiscard.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "放弃修改" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("rejects custom role names with surrounding whitespace", async () => {
    const user = userEvent.setup();
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()) } as TauriClient);
    await screen.findByText("default", { exact: true });
    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "角色名称" }), " analyst ");
    await user.click(within(dialog).getByRole("button", { name: "添加" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("角色名称不能为空");
    expect(screen.queryByText("analyst", { exact: true })).not.toBeInTheDocument();
  });

  it("creates a custom role only after a complete selector, then supports edit rename and delete", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    const overview = overviewDto({ roles: [...overviewDto().roles, { id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    await screen.findByText("default", { exact: true });

    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "角色名称" }), "analyst");
    const provider = within(dialog).getByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    const model = within(dialog).getByRole("combobox", { name: "模型" });
    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "gpt-5.6-sol" }));
    await user.click(within(dialog).getByRole("button", { name: "添加" }));
    const analyst = await screen.findByText("analyst", { exact: true });
    expect(analyst).toBeVisible();

    await user.click(screen.getByRole("button", { name: "角色操作 analyst" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const editDialog = await screen.findByRole("dialog");
    expect(within(editDialog).getByRole("textbox", { name: "角色名称" })).toHaveValue("analyst");
    await user.click(within(editDialog).getByRole("button", { name: "保存" }));

    await user.click(screen.getByRole("button", { name: "角色操作 analyst" }));
    await user.click(screen.getByRole("menuitem", { name: "改名" }));
    const renameDialog = await screen.findByRole("dialog");
    const renameInput = within(renameDialog).getByRole("textbox", { name: "角色名称" });
    await user.clear(renameInput);
    await user.type(renameInput, "reviewer");
    await user.click(within(renameDialog).getByRole("button", { name: "保存" }));
    expect(await screen.findByText("reviewer", { exact: true })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const duplicateDialog = await screen.findByRole("dialog");
    await user.type(within(duplicateDialog).getByRole("textbox", { name: "角色名称" }), "researcher");
    await user.click(within(duplicateDialog).getByRole("combobox", { name: "Provider" }));
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    await user.click(within(duplicateDialog).getByRole("combobox", { name: "模型" }));
    await user.click(await screen.findByRole("option", { name: "gpt-5.6-sol" }));
    await user.click(within(duplicateDialog).getByRole("button", { name: "添加" }));
    expect(await within(duplicateDialog).findByRole("alert")).toHaveTextContent("角色名称已存在");
    await user.click(within(duplicateDialog).getByRole("button", { name: "取消" }));
    const duplicateDiscard = await screen.findByRole("heading", { name: "有未保存的修改" });
    await user.click(within(duplicateDiscard.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "放弃修改" }));

    await user.click(screen.getByRole("button", { name: "角色操作 reviewer" }));
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const confirmation = await screen.findByRole("heading", { name: "删除自定义角色？" });
    await user.click(within(confirmation.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "删除角色" }));
    expect(screen.queryByText("reviewer", { exact: true })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存修改" })).toBeDisabled();
  });
  it("persists creating a custom role through the global save", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()), saveModelRoles } as TauriClient);
    await screen.findByText("default", { exact: true });

    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "角色名称" }), "analyst");
    await user.click(within(dialog).getByRole("combobox", { name: "Provider" }));
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    await user.click(within(dialog).getByRole("combobox", { name: "模型" }));
    await user.click(await screen.findByRole("option", { name: "gpt-5.6-sol" }));
    await user.click(within(dialog).getByRole("button", { name: "添加" }));
    await user.click(screen.getByRole("button", { name: "保存修改" }));

    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith({
      openedConfigHash: "config-hash",
      changes: [{ kind: "create", roleId: "analyst", providerId: "dnslin", modelId: "gpt-5.6-sol" }],
    }));
  });

  it("persists editing a custom role selector through the global save", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    const overview = overviewDto({ roles: [{ id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    await screen.findByText("researcher", { exact: true });

    await user.click(screen.getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("combobox", { name: "Thinking" }));
    await user.click(await screen.findByRole("option", { name: "high" }));
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await user.click(screen.getByRole("button", { name: "保存修改" }));

    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith({
      openedConfigHash: "config-hash",
      changes: [{ kind: "set", roleId: "researcher", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: "high" }],
    }));
  });

  it("persists renaming a custom role through the global save", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    const overview = overviewDto({ roles: [{ id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    await screen.findByText("researcher", { exact: true });

    await user.click(screen.getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "改名" }));
    const dialog = await screen.findByRole("dialog");
    const roleInput = within(dialog).getByRole("textbox", { name: "角色名称" });
    await user.clear(roleInput);
    await user.type(roleInput, "reviewer");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await user.click(screen.getByRole("button", { name: "保存修改" }));

    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith({
      openedConfigHash: "config-hash",
      changes: [{ kind: "rename", roleId: "researcher", newRoleId: "reviewer", providerId: "dnslin", modelId: "gpt-5.6-sol" }],
    }));
  });

  it("rejects empty custom role values and locks the whole page for advanced roles", async () => {
    const user = userEvent.setup();
    const openTargetConfigurationDirectory = vi.fn(async () => undefined);
    const advanced = overviewDto({ roles: [{ id: "default", status: "advanced", selector: null, providerId: null, modelId: null, thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(advanced), openTargetConfigurationDirectory } as TauriClient);
    await screen.findByText("default", { exact: true });
    expect(screen.getByText("角色配置为只读")).toBeVisible();
    expect(screen.getByRole("button", { name: "新增自定义角色" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Provider default" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "打开配置目录" }));
    expect(openTargetConfigurationDirectory).toHaveBeenCalledWith("/usr/local/bin/omp");

    cleanup();
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()) } as TauriClient);
    await screen.findByText("default", { exact: true });
    await user.click(screen.getByRole("button", { name: "新增自定义角色" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "角色名称" }), "empty");
    await user.click(within(dialog).getByRole("button", { name: "添加" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("请选择普通");
    expect(screen.queryByText("empty", { exact: true })).not.toBeInTheDocument();
  });

  it("locks role editing when the overview marks configuration read-only", async () => {
    const overview = overviewDto({ state: "read-only", rolesEditable: false, rolesAssignable: false, rolesReadOnlyReason: "当前配置业务结构无法识别，只能查看；OMP Switch 不会修改未知结构。", readOnlyReason: "当前配置业务结构无法识别，只能查看；OMP Switch 不会修改未知结构。" });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    await screen.findByText("default", { exact: true });
    expect(screen.getByText("当前配置业务结构无法识别，只能查看；OMP Switch 不会修改未知结构。")).toBeVisible();
    expect(screen.getByRole("button", { name: "新增自定义角色" })).toBeDisabled();
  });

  it("shows invalid simple role references without locking repair controls", async () => {
    const user = userEvent.setup();
    const overview = overviewDto({
      roles: [
        { id: "default", status: "provider-missing", selector: "missing/gpt", providerId: "missing", modelId: "gpt", thinkingLevel: null },
        { id: "smol", status: "model-missing", selector: "dnslin/old", providerId: "dnslin", modelId: "old", thinkingLevel: null },
        { id: "slow", status: "incomplete", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null },
        { id: "researcher", status: "incomplete", selector: "dnslin/old", providerId: "dnslin", modelId: "old", thinkingLevel: null },
      ],
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    expect(await screen.findByText("Provider 不存在")).toBeVisible();
    expect(screen.getByText("模型不存在")).toBeVisible();
    expect(screen.getAllByText("模型配置不完整")).toHaveLength(2);
    expect(screen.getByRole("combobox", { name: "Provider default" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "Thinking default" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Thinking researcher" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const editDialog = await screen.findByRole("dialog");
    await user.click(within(editDialog).getByRole("button", { name: "保存" }));
    expect(await within(editDialog).findByRole("alert")).toHaveTextContent("请选择普通");
    await user.click(within(editDialog).getByRole("button", { name: "取消" }));
  });
  it("assigns a model whose ID matches the empty option sentinel", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const specialModel: OverviewModel = { ...base.models[0], id: "__none__" };
    const specialProvider: OverviewProvider = { ...base.providers[0], modelCount: 1, models: [specialModel] };
    const overview = overviewDto({
      counts: { providerCount: 1, modelCount: 1, roleCount: 1 },
      providers: [specialProvider],
      models: [specialModel],
      roles: [{ id: "default", status: "unconfigured", selector: null, providerId: null, modelId: null, thinkingLevel: null }],
    });
    const saveModelRoles = vi.fn().mockResolvedValue({ changedRoleCount: 1 });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview), saveModelRoles } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    await user.click(within(row).getByRole("combobox", { name: "模型 default" }));
    await user.click(await screen.findByRole("option", { name: "__none__" }));
    expect(within(row).getByRole("combobox", { name: "模型 default" })).toHaveTextContent("__none__");
    await user.click(screen.getByRole("button", { name: "保存修改" }));
    await waitFor(() => expect(saveModelRoles).toHaveBeenCalledWith(expect.objectContaining({ changes: [expect.objectContaining({ modelId: "__none__" })] })));
  });
  it("does not offer selector-unsafe model definitions", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const unsafeModel: OverviewModel = { ...base.models[0], id: "gpt,5" };
    const unsafeProvider: OverviewProvider = { ...base.providers[0], models: [unsafeModel] };
    const overview = overviewDto({
      providers: [unsafeProvider],
      models: [unsafeModel],
      roles: [{ id: "default", status: "unconfigured", selector: null, providerId: null, modelId: null, thinkingLevel: null }],
      rolesAssignable: true,
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    expect(screen.queryByRole("option", { name: "dnslin" })).not.toBeInTheDocument();
  });
  it("keeps Rust-accepted format characters available in model selectors", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const formatModel: OverviewModel = { ...base.models[0], id: "gpt\u200d5" };
    const formatProvider: OverviewProvider = { ...base.providers[0], models: [formatModel] };
    const overview = overviewDto({
      providers: [formatProvider],
      models: [formatModel],
      roles: [{ id: "default", status: "unconfigured", selector: null, providerId: null, modelId: null, thinkingLevel: null }],
      rolesAssignable: true,
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    expect(screen.getByRole("option", { name: "dnslin" })).toBeVisible();
  });
  it("does not offer models whose ID is only a Thinking suffix", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const emptyBaseModel: OverviewModel = { ...base.models[0], id: ":high" };
    const emptyBaseProvider: OverviewProvider = { ...base.providers[0], models: [emptyBaseModel] };
    const overview = overviewDto({
      providers: [emptyBaseProvider],
      models: [emptyBaseModel],
      roles: [{ id: "default", status: "unconfigured", selector: null, providerId: null, modelId: null, thinkingLevel: null }],
      rolesAssignable: true,
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    expect(screen.queryByRole("option", { name: "dnslin" })).not.toBeInTheDocument();
  });



  it("renders mixed-case role references in controlled selects", async () => {
    const base = overviewDto();
    const mixedModel: OverviewModel = { ...base.models[0], providerId: "Dnslin", id: "GPT-5.6-Luna" };
    const mixedProvider: OverviewProvider = { ...base.providers[0], id: "Dnslin", modelCount: 1, models: [mixedModel] };
    const overview = overviewDto({
      counts: { providerCount: 1, modelCount: 1, roleCount: 1 },
      providers: [mixedProvider],
      models: [mixedModel],
      roles: [{ id: "default", status: "configured", selector: "dnslin/gpt-5.6-luna", providerId: "Dnslin", modelId: "GPT-5.6-Luna", thinkingLevel: null }],
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    expect(within(row).getByRole("combobox", { name: "Provider default" })).toHaveTextContent("Dnslin");
    expect(within(row).getByRole("combobox", { name: "模型 default" })).toHaveTextContent("GPT-5.6-Luna");
  });
  it("allows a renamed custom role to return to its original name", async () => {
    const user = userEvent.setup();
    const base = overviewDto();
    const overview = overviewDto({ roles: [...base.roles, { id: "researcher", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    await screen.findByText("researcher", { exact: true });
    await user.click(screen.getByRole("button", { name: "角色操作 researcher" }));
    await user.click(screen.getByRole("menuitem", { name: "改名" }));
    const firstDialog = await screen.findByRole("dialog");
    const firstInput = within(firstDialog).getByRole("textbox", { name: "角色名称" });
    await user.clear(firstInput);
    await user.type(firstInput, "reviewer");
    await user.click(within(firstDialog).getByRole("button", { name: "保存" }));
    await user.click(screen.getByRole("button", { name: "角色操作 reviewer" }));
    await user.click(screen.getByRole("menuitem", { name: "改名" }));
    const secondDialog = await screen.findByRole("dialog");
    const secondInput = within(secondDialog).getByRole("textbox", { name: "角色名称" });
    await user.clear(secondInput);
    await user.type(secondInput, "researcher");
    await user.click(within(secondDialog).getByRole("button", { name: "保存" }));
    expect(await screen.findByText("researcher", { exact: true })).toBeVisible();
  });

  it("keeps distinct Unicode model IDs distinct in role selectors", async () => {
    const base = overviewDto();
    const upperModel: OverviewModel = { ...base.models[0], id: "Ä" };
    const lowerModel: OverviewModel = { ...base.models[0], id: "ä" };
    const provider: OverviewProvider = { ...base.providers[0], modelCount: 2, models: [upperModel, lowerModel] };
    const overview = overviewDto({
      counts: { providerCount: 1, modelCount: 2, roleCount: 1 },
      providers: [provider],
      models: [upperModel, lowerModel],
      roles: [{ id: "default", status: "configured", selector: "dnslin/ä", providerId: "dnslin", modelId: "ä", thinkingLevel: null }],
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    expect(within(row).getByRole("combobox", { name: "模型 default" })).toHaveTextContent("ä");
  });
  it("initializes configured roles from structured fields when selectors are redacted", async () => {
    const base = overviewDto();
    const model: OverviewModel = { ...base.models[0], providerId: "sk-local", id: "gpt-5.6-sol" };
    const provider: OverviewProvider = { ...base.providers[0], id: "sk-local", models: [model] };
    const overview = overviewDto({
      providers: [provider],
      models: [model],
      roles: [{ id: "default", status: "configured", selector: "[已脱敏]", providerId: "sk-local", modelId: "gpt-5.6-sol", thinkingLevel: "high" }],
    });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    expect(within(row).getByRole("combobox", { name: "Provider default" })).toHaveTextContent("sk-local");
    expect(within(row).getByRole("combobox", { name: "模型 default" })).toHaveTextContent("gpt-5.6-sol");
    expect(within(row).getByRole("combobox", { name: "Thinking default" })).toHaveTextContent("high");
    expect(screen.getByRole("button", { name: "保存修改" })).toBeDisabled();
  });





  it("protects partial role assignments as dirty drafts", async () => {
    const user = userEvent.setup();
    const { router } = renderRoute("/roles", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ roles: [{ id: "default", status: "unconfigured", selector: null, providerId: null, modelId: null, thinkingLevel: null }, { id: "task", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }] })),
    } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    expect(screen.getByText("有未保存的修改")).toBeVisible();

    void router.navigate("/overview");
    const discardHeading = await screen.findByRole("heading", { name: "有未保存的修改" });
    await user.click(within(discardHeading.closest('[role="dialog"]') as HTMLElement).getByRole("button", { name: "取消" }));
    expect(router.state.location.pathname).toBe("/roles");
  });

  it("does not serialize a partial provider reassignment as clear", async () => {
    const user = userEvent.setup();
    const saveModelRoles = vi.fn(async () => ({ changedRoleCount: 0 }));
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewSelectionDto()), saveModelRoles } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Provider default" }));
    await user.click(await screen.findByRole("option", { name: "anthropic" }));

    expect(screen.getByText("有未保存的修改")).toBeVisible();
    expect(screen.getByRole("button", { name: "保存修改" })).toBeDisabled();
    expect(screen.getByText("待保存")).toBeVisible();
    expect(saveModelRoles).not.toHaveBeenCalled();
  });

  it("keeps dirty role edits after a save conflict until reload is confirmed", async () => {
    const user = userEvent.setup();
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto()));
    const saveModelRoles = vi.fn().mockRejectedValue({ code: "config-hash-conflict", message: "config.yml 已被外部修改。", action: "请重新读取配置。" });
    renderRoute("/roles", { ...unavailableClient, getOverviewLoad, saveModelRoles } as TauriClient);
    const row = (await screen.findByText("default", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("combobox", { name: "Thinking default" }));
    await user.click(await screen.findByRole("option", { name: "high" }));
    expect(screen.getByText("待保存")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "保存修改" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("配置冲突");
    expect(screen.getByText("有未保存的修改")).toBeVisible();
    expect(saveModelRoles).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    const reloadDialog = screen.getByRole("dialog");
    expect(reloadDialog).toHaveTextContent("重新读取会丢弃当前未保存的角色修改。");
    fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    expect(saveModelRoles).toHaveBeenCalledTimes(1);
    await user.click(within(reloadDialog).getByRole("button", { name: "取消" }));
    expect(screen.getByText("有未保存的修改")).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "重新读取" }));
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(screen.queryByText("待保存")).not.toBeInTheDocument();
      expect(screen.queryByText("有未保存的修改")).not.toBeInTheDocument();
    });
  });


});

function overviewDto(overrides: Partial<OverviewDto> = {}): OverviewDto {
  const model: OverviewModel = {
    providerId: "dnslin",
    id: "gpt-5.6-sol",
    name: "Sol",
    effectiveApi: "openai-responses",
    apiSource: "model",
    hasBaseUrlOverride: false,
    input: ["text", "image"],
    reasoning: true,
    contextWindow: 356000,
    maxTokens: 32768,
    complete: true,
    unsupportedProtocol: false,
    status: "normal",
    editable: true,
    referenceCount: 0,
    referencePaths: [],
    roleReferencePaths: [],
    otherReferencePaths: [],
    readOnlyReason: null,
  };
  return {
    state: "normal",
    omp: { status: "connected", executablePath: "/usr/local/bin/omp", version: "17.4.1" },
    targetConfiguration: targetConfiguration(),
    files: {
      models: { canonicalPath: "/Users/username/.omp/agent/models.yml", resolvedPath: "/Users/username/.omp/agent/models.yml", status: "normal", contentHash: "models-hash" },
      config: { canonicalPath: "/Users/username/.omp/agent/config.yml", resolvedPath: "/Users/username/.omp/agent/config.yml", status: "normal", contentHash: "config-hash" },
    },
    counts: { providerCount: 1, modelCount: 1, roleCount: 2 },
    providers: [{ id: "dnslin", name: "Local", baseUrl: "https://example.com", defaultApi: "openai-responses", authMode: "api-key", hasApiKey: true, modelCount: 1, classification: "custom", editable: true, readOnlyReason: null, roleReferencePaths: [], otherReferencePaths: [], models: [model] }],
    models: [model],
    roles: [{ id: "default", status: "configured", selector: "dnslin/gpt-5.6-sol:max", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: "max" }, { id: "task", status: "configured", selector: "dnslin/gpt-5.6-sol", providerId: "dnslin", modelId: "gpt-5.6-sol", thinkingLevel: null }],
    rolesEditable: true,
    rolesAssignable: true,
    rolesReadOnlyReason: null,
    emptyReason: null,
    nextAction: null,
    readOnlyReason: null,
    ...overrides,
  };
}

function overviewSelectionDto(): OverviewDto {
  const primary = overviewDto();
  const anthropicModel: OverviewModel = {
    ...primary.models[0],
    providerId: "anthropic",
    id: "claude-sonnet-4",
    name: "Claude Sonnet 4",
    effectiveApi: "anthropic-messages",
    apiSource: "provider",
    input: ["text"],
    reasoning: false,
    contextWindow: 200000,
    maxTokens: 8192,
  };
  const anthropicProvider: OverviewProvider = {
    ...primary.providers[0],
    id: "anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    defaultApi: "anthropic-messages",
    modelCount: 1,
    models: [anthropicModel],
  };
  return overviewDto({
    counts: { providerCount: 2, modelCount: 2, roleCount: 2 },
    providers: [primary.providers[0], anthropicProvider],
    models: [primary.models[0], anthropicModel],
  });
}

describe("Overview page seam", () => {
  it("renders the normal overview, safe counts, and connected sidebar status", async () => {
    const overview = overviewDto();

    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });

    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
    expect(screen.getByText("查看当前配置状态并快速验证模型连接。")).toBeVisible();
    expect(screen.getByText("自定义 Provider")).toBeVisible();
    expect(screen.getAllByText("1")[0]).toBeVisible();
    expect(screen.getByLabelText(/OMP 已连接.*v17\.4\.1/)).toBeVisible();
    expect(screen.getAllByText("/Users/username/.omp/agent")[0]).toBeVisible();
    expect(screen.queryByText("super-secret-api-key")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "测试模型" })).toBeEnabled();
    expect(screen.getByRole("region", { name: "快速测试" })).toHaveTextContent(/Text\s+·\s+Image\s+·\s+Reasoning/);
    expect(screen.getByRole("region", { name: "测试结果" })).toBeVisible();
    expect(screen.getByText("尚未测试")).toBeVisible();
  });
  it("explains when the final endpoint cannot be constructed", async () => {
    const overview = overviewDto();
    const provider = {
      ...overview.providers[0],
      baseUrl: "[配置地址因无法解析而已脱敏]",
    };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider] })),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent("Provider Base URL 无效或已脱敏");
  });
  it("rejects a non-HTTP Provider Base URL from the final endpoint preview", async () => {
    const overview = overviewDto();
    const provider = {
      ...overview.providers[0],
      baseUrl: "ftp://example.com",
    };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider] })),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent("Provider Base URL 必须使用 HTTP(S)");
    expect(panel).not.toHaveTextContent("ftp://example.com/v1/responses");
  });
  it("exposes Provider and Model choices as accessible comboboxes", async () => {
    const user = userEvent.setup();
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
    });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    const model = screen.getByRole("combobox", { name: "模型" });
    expect(provider).toHaveTextContent("dnslin");
    expect(model).toHaveTextContent("gpt-5.6-sol");

    await user.click(provider);
    expect(await screen.findByRole("option", { name: "dnslin" })).toBeVisible();
    expect(screen.getByRole("option", { name: "anthropic" })).toBeVisible();
    await user.keyboard("{Escape}");

    await user.click(model);
    expect(await screen.findByRole("option", { name: "gpt-5.6-sol" })).toBeVisible();
  });
  it("exposes an overlong Model Stable ID through the accessible option", async () => {
    const user = userEvent.setup();
    const overview = overviewDto();
    const longModelId = "model-with-an-overlong-stable-id-that-must-not-expand-the-approved-overview-select-width-0123456789";
    const longModel = { ...overview.models[0], id: longModelId };
    const provider = { ...overview.providers[0], models: [longModel] };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider], models: [longModel] })),
    });

    const model = await screen.findByRole("combobox", { name: "模型" });
    await user.click(model);
    const option = await screen.findByRole("option", { name: longModelId });
    expect(option).toHaveTextContent(longModelId);
  });
  it("disables empty Provider and Model comboboxes with explicit placeholders", async () => {
    const overview = overviewDto({
      state: "empty",
      counts: { providerCount: 0, modelCount: 0, roleCount: 0 },
      providers: [],
      models: [],
      roles: [],
      emptyReason: "还没有可管理的自定义 Provider。",
      nextAction: "创建一个 Provider，并同时配置它的第一个模型。",
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    const model = screen.getByRole("combobox", { name: "模型" });
    expect(provider).toBeDisabled();
    expect(provider).toHaveTextContent("暂无 Provider");
    expect(model).toBeDisabled();
    expect(model).toHaveTextContent("暂无模型");
  });
  it("clears an incompatible Model on Provider change and persists complete settings", async () => {
    const user = userEvent.setup();
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "anthropic" }));

    const panel = screen.getByRole("region", { name: "快速测试" });
    const model = screen.getByRole("combobox", { name: "模型" });
    expect(model).toHaveTextContent("请选择模型");
    expect(panel).toHaveTextContent(/有效协议—最终地址—能力—Context Window—/);
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(1));
    expect(saveUiSettings).toHaveBeenNthCalledWith(1, {
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: null,
    });

    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "claude-sonnet-4" }));
    expect(panel).toHaveTextContent(/anthropic-messages\s+·\s+Provider 默认值/);
    expect(panel).toHaveTextContent("https://api.anthropic.com/v1/messages");
    expect(panel).toHaveTextContent("Text");
    expect(panel).toHaveTextContent("200,000");
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(2));
    expect(saveUiSettings).toHaveBeenNthCalledWith(2, {
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
    });
  });
  it("clears the Model when switching Provider even if the new Provider reuses the Model ID", async () => {
    const user = userEvent.setup();
    const overview = overviewSelectionDto();
    const reusedModel = { ...overview.models[0], providerId: "anthropic", id: overview.models[0].id, name: "Other Provider Model" };
    const reusedProvider = { ...overview.providers[1], models: [reusedModel], modelCount: 1 };
    const updated = overviewDto({
      providers: [overview.providers[0], reusedProvider],
      models: [overview.models[0], reusedModel],
    });
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(updated),
      getUiSettings: async () => ({
        ompExecutablePath: null,
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: overview.models[0].id,
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "anthropic" }));

    expect(screen.getByRole("combobox", { name: "模型" })).toHaveTextContent("请选择模型");
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledWith({
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: null,
    }));
  });
  it("serializes rapid Provider and Model saves", async () => {
    const user = userEvent.setup();
    const firstSave = deferred<Awaited<ReturnType<TauriClient["saveUiSettings"]>>>();
    const secondSave = deferred<Awaited<ReturnType<TauriClient["saveUiSettings"]>>>();
    const saveUiSettings = vi.fn<TauriClient["saveUiSettings"]>()
      .mockImplementationOnce(() => firstSave.promise)
      .mockImplementationOnce(() => secondSave.promise);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => ({
        ompExecutablePath: null,
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "anthropic" }));
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(1));

    const model = screen.getByRole("combobox", { name: "模型" });
    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "claude-sonnet-4" }));
    expect(model).toHaveTextContent("claude-sonnet-4");
    expect(saveUiSettings).toHaveBeenCalledTimes(1);

    firstSave.reject({
      code: "settings-write-failed",
      message: "无法保存快速测试选择",
      action: "请重试。",
    });
    expect(await screen.findByText("无法保存快速测试选择")).toBeVisible();
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(2));
    expect(saveUiSettings).toHaveBeenNthCalledWith(2, {
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
    });
    secondSave.resolve({
      ompExecutablePath: null,
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
      modelTestCostNoticeAccepted: true,
    });
  });
  it("serializes selection saves across Overview remounts", async () => {
    const user = userEvent.setup();
    const firstSave = deferred<Awaited<ReturnType<TauriClient["saveUiSettings"]>>>();
    const secondSave = deferred<Awaited<ReturnType<TauriClient["saveUiSettings"]>>>();
    const thirdSave = deferred<Awaited<ReturnType<TauriClient["saveUiSettings"]>>>();
    const saveUiSettings = vi.fn<TauriClient["saveUiSettings"]>()
      .mockImplementationOnce(() => firstSave.promise)
      .mockImplementationOnce(() => secondSave.promise)
      .mockImplementationOnce(() => thirdSave.promise);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => ({
        ompExecutablePath: null,
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "anthropic" }));
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(1));
    const model = screen.getByRole("combobox", { name: "模型" });
    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "claude-sonnet-4" }));
    expect(saveUiSettings).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("link", { name: "Providers" }));
    expect(await screen.findByRole("heading", { name: "Providers" })).toBeVisible();
    await user.click(screen.getByRole("link", { name: "概览" }));
    const remountedProvider = await screen.findByRole("combobox", { name: "Provider" });
    expect(remountedProvider).toHaveTextContent("anthropic");
    await user.click(remountedProvider);
    await user.click(await screen.findByRole("option", { name: "dnslin" }));
    expect(saveUiSettings).toHaveBeenCalledTimes(1);

    firstSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "anthropic", selectedModelId: null, modelTestCostNoticeAccepted: true });
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(2));
    expect(saveUiSettings).toHaveBeenNthCalledWith(2, {
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
    });
    secondSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "anthropic", selectedModelId: "claude-sonnet-4", modelTestCostNoticeAccepted: true });
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(3));
    expect(saveUiSettings).toHaveBeenNthCalledWith(3, {
      theme: "dark",
      selectedProviderId: "dnslin",
      selectedModelId: null,
    });
    thirdSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "dnslin", selectedModelId: null, modelTestCostNoticeAccepted: true });
  });
  it("waits for UI settings hydration before showing overview content", async () => {
    const settings = deferred<Awaited<ReturnType<TauriClient["getUiSettings"]>>>();
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto()));
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      getUiSettings: () => settings.promise,
      saveUiSettings,
    });

    await screen.findByRole("heading", { name: "概览" });
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("status", { name: "正在读取配置" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "快速测试" })).not.toBeInTheDocument();

    settings.resolve({
      ompExecutablePath: null,
      theme: "system",
      selectedProviderId: null,
      selectedModelId: null,
      modelTestCostNoticeAccepted: false,
    });

    expect(await screen.findByRole("region", { name: "快速测试" })).toBeVisible();
    expect(saveUiSettings).not.toHaveBeenCalled();
  });
  it("restores a saved Provider and Model pair without saving during hydration", async () => {
    const overview = overviewSelectionDto();
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overview),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "anthropic",
        selectedModelId: "claude-sonnet-4",
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveTextContent("anthropic");
    expect(screen.getByRole("combobox", { name: "模型" })).toHaveTextContent("claude-sonnet-4");
    expect(panel).toHaveTextContent("anthropic");
    expect(panel).toHaveTextContent("claude-sonnet-4");
    expect(panel).toHaveTextContent(/anthropic-messages\s+·\s+Provider 默认值/);
    expect(panel).toHaveTextContent("https://api.anthropic.com/v1/messages");
    expect(panel).toHaveTextContent("200,000");
    expect(saveUiSettings).not.toHaveBeenCalled();
  });
  it("preserves a saved Provider with an intentionally empty Model", async () => {
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => ({
        ompExecutablePath: null,
        theme: "system",
        selectedProviderId: "anthropic",
        selectedModelId: null,
        modelTestCostNoticeAccepted: false,
      }),
      saveUiSettings,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent("anthropic");
    expect(panel).toHaveTextContent("请选择模型");
    expect(panel).not.toHaveTextContent("claude-sonnet-4");
    expect(saveUiSettings).not.toHaveBeenCalled();
  });
  it.each([
    ["stale Model", "dnslin", "claude-sonnet-4", "dnslin", null],
    ["missing Provider", "removed-provider", "claude-sonnet-4", null, null],
    ["Model without Provider", null, "claude-sonnet-4", null, null],
  ] as const)("cleans a %s selection once under StrictMode", async (_case, selectedProviderId, selectedModelId, expectedProviderId, expectedModelId) => {
    const saveUiSettings = vi.fn(unavailableClient.saveUiSettings);
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto()),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId,
        selectedModelId,
        modelTestCostNoticeAccepted: true,
      }),
      saveUiSettings,
    }, true);

    expect(await screen.findByText("之前选择的模型已不存在，请重新选择。")).toBeVisible();
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(1));
    expect(saveUiSettings).toHaveBeenCalledWith({
      theme: "dark",
      selectedProviderId: expectedProviderId,
      selectedModelId: expectedModelId,
    });
    expect(screen.getAllByText("之前选择的模型已不存在，请重新选择。")).toHaveLength(1);
  });
  it("exercises the duplicate overview load caused by React StrictMode", async () => {
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto()));
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad }, true);

    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));
  });
  it.each([
    ["openai-responses", "https://example.com/v1/responses?region=us"],
    ["anthropic-messages", "https://example.com/v1/messages?region=us"],
    ["google-generative-ai", "https://example.com/v1/models/gpt-5.6-sol:streamGenerateContent?region=us&alt=sse"],
  ] as const)("builds the %s preview path before the base URL query", async (effectiveApi, expectedAddress) => {
    const overview = overviewDto();
    const model = { ...overview.models[0], effectiveApi };
    const provider = { ...overview.providers[0], baseUrl: "https://example.com/v1?region=us", models: [model] };
    const updated = overviewDto({ providers: [provider], models: [model] });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(updated) });

    expect(await screen.findByRole("region", { name: "快速测试" })).toHaveTextContent(expectedAddress);
  });
  it("does not preview a model-level Base URL override", async () => {
    const overview = overviewDto();
    const overrideModel: OverviewModel = {
      ...overview.models[0],
      hasBaseUrlOverride: true,
      complete: false,
      editable: false,
      readOnlyReason: "Model definition 包含模型级 Base URL 覆盖，只能查看。",
    };
    const provider = { ...overview.providers[0], models: [overrideModel], modelCount: 1 };
    const updated = overviewDto({ providers: [provider], models: [overrideModel] });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(updated) });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent("模型级 Base URL 覆盖不可安全展示");
    expect(panel).not.toHaveTextContent("https://example.com/v1/responses");
  });

  it("does not preselect a read-only model in quick test", async () => {
    const overview = overviewDto();
    const readOnlyModel = {
      ...overview.models[0],
      providerId: "missing",
      id: "invalid-model",
      editable: false,
      readOnlyReason: "Provider 必须包含有效的 HTTP(S) Base URL。",
    };
    const readOnlyProvider = {
      ...overview.providers[0],
      id: "missing",
      editable: false,
      classification: "unsupported" as const,
      readOnlyReason: "Provider 必须包含有效的 HTTP(S) Base URL。",
      models: [readOnlyModel],
    };
    const updated = overviewDto({
      counts: { providerCount: 1, modelCount: 2, roleCount: 0 },
      providers: [readOnlyProvider, overview.providers[0]],
      models: [readOnlyModel, overview.models[0]],
      roles: [],
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(updated) });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent("dnslin");
    expect(panel).toHaveTextContent("gpt-5.6-sol");
    expect(panel).not.toHaveTextContent("missing");
  });
  it("allows read-only incomplete projections to be selected for safe summaries", async () => {
    const user = userEvent.setup();
    const overview = overviewSelectionDto();
    const readOnlyModel: OverviewModel = {
      ...overview.models[0],
      providerId: "legacy",
      id: "legacy-preview",
      effectiveApi: "openai-completions",
      apiSource: "model",
      input: ["text", "image"],
      reasoning: true,
      contextWindow: 64000,
      complete: false,
      editable: false,
      readOnlyReason: "模型包含当前版本不支持的字段。",
    };
    const readOnlyProvider: OverviewProvider = {
      ...overview.providers[0],
      id: "legacy",
      name: "Legacy",
      baseUrl: "https://legacy.example/v1",
      defaultApi: "openai-completions",
      modelCount: 1,
      classification: "advanced" as const,
      editable: false,
      readOnlyReason: "Provider 包含当前版本不支持的字段。",
      models: [readOnlyModel],
    };
    const updated = overviewDto({
      counts: { providerCount: 3, modelCount: 3, roleCount: 2 },
      providers: [...overview.providers, readOnlyProvider],
      models: [...overview.models, readOnlyModel],
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(updated) });

    const provider = await screen.findByRole("combobox", { name: "Provider" });
    await user.click(provider);
    await user.click(await screen.findByRole("option", { name: "legacy" }));
    const model = screen.getByRole("combobox", { name: "模型" });
    await user.click(model);
    await user.click(await screen.findByRole("option", { name: "legacy-preview" }));

    const panel = screen.getByRole("region", { name: "快速测试" });
    expect(panel).toHaveTextContent(/openai-completions\s+·\s+模型指定/);
    expect(panel).toHaveTextContent("https://legacy.example/v1/chat/completions");
    expect(panel).toHaveTextContent(/Text\s+·\s+Image\s+·\s+Reasoning/);
    expect(panel).toHaveTextContent("64,000");
    expect(screen.getByRole("button", { name: "测试模型" })).toBeDisabled();
  });

  it("keeps an alternate YAML file in warning status", async () => {
    const overview = overviewDto({
      files: {
        models: { ...overviewDto().files.models, status: "canonical-with-alternate" },
        config: overviewDto().files.config,
      },
    });
    const { container } = renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });
    expect(await screen.findByText(/models\.yml\s+正常 · 有 \.yaml/)).toBeVisible();
    expect(container.querySelectorAll(".overview-file-status-icon--warning")).toHaveLength(1);
  });

  it.each([
    [overviewDto({ state: "empty", counts: { providerCount: 0, modelCount: 0, roleCount: 0 }, providers: [], models: [], roles: [], emptyReason: "还没有可管理的自定义 Provider。", nextAction: "创建一个 Provider，并同时配置它的第一个模型。" }), "还没有可管理的自定义 Provider", "创建一个 Provider，并同时配置它的第一个模型"],
    [overviewDto({ state: "read-only", readOnlyReason: "当前配置只能查看；OMP Switch 不会修改 .yaml 或不可写文件。" }), "配置只读", "当前配置只能查看"],
  ] as const)("renders overview %s state", async (overview, visibleText, detailText) => {
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });
    expect((await screen.findAllByText(new RegExp(visibleText)))[0]).toBeVisible();
    expect(screen.getByText(new RegExp(detailText))).toBeVisible();
  });
  it("distinguishes a Provider-management read-only state from a read-only target", async () => {
    const base = overviewDto();
    const overview = overviewDto({
      state: "read-only",
      targetConfiguration: targetConfiguration(),
      providers: [{
        ...base.providers[0],
        classification: "unavailable",
        editable: false,
        readOnlyReason: "当前 OMP 版本没有匹配的 bundled Provider 清单。",
      }],
      readOnlyReason: "当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。",
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });

    expect(await screen.findByText("没有可编辑的自定义 Provider")).toBeVisible();
    expect(screen.getByText(/bundled Provider 清单/)).toBeVisible();
  });
  it.each([
    [overviewDto({ state: "empty", counts: { providerCount: 0, modelCount: 0, roleCount: 0 }, providers: [], models: [], roles: [], emptyReason: "还没有可管理的自定义 Provider。", nextAction: "创建一个 Provider，并同时配置它的第一个模型。" }), "新增 Provider"],
    [overviewDto({ state: "read-only", readOnlyReason: "当前配置只能查看；OMP Switch 不会修改 .yaml 或不可写文件。" }), "查看 Providers"],
  ] as const)("offers the required Overview state action for %s", async (overview, actionLabel) => {
    const user = userEvent.setup();
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });

    await user.click(await screen.findByRole("link", { name: actionLabel }));
    expect(await screen.findByRole("heading", { name: "Providers" })).toBeVisible();
  });
  it("renders missing files without success indicators", async () => {
    const overview = overviewDto({
      state: "empty",
      files: {
        models: { ...overviewDto().files.models, resolvedPath: null, status: "missing", contentHash: null },
        config: { ...overviewDto().files.config, resolvedPath: null, status: "missing", contentHash: null },
      },
      counts: { providerCount: 0, modelCount: 0, roleCount: 0 },
      providers: [],
      models: [],
      roles: [],
      emptyReason: "还没有可管理的自定义 Provider。",
      nextAction: "创建一个 Provider，并同时配置它的第一个模型。",
    });
    const { container } = renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });
    expect(await screen.findByText(/models\.yml\s+缺失/)).toBeVisible();
    expect(screen.getByText(/config\.yml\s+缺失/)).toBeVisible();
    const strip = screen.getByRole("region", { name: "配置同步状态" });
    expect(strip.querySelectorAll(".overview-file-status-icon--success")).toHaveLength(0);
    expect(strip.querySelectorAll(".overview-file-status-icon--warning")).toHaveLength(2);
    expect(container.querySelectorAll(".overview-file-status-icon--danger")).toHaveLength(0);
  });
  it("routes missing configuration files to first-time setup", async () => {
    const user = userEvent.setup();
    const overview = overviewDto({
      state: "empty",
      files: {
        models: { ...overviewDto().files.models, resolvedPath: null, status: "missing", contentHash: null },
        config: { ...overviewDto().files.config, resolvedPath: null, status: "missing", contentHash: null },
      },
      counts: { providerCount: 0, modelCount: 0, roleCount: 0 },
      providers: [],
      models: [],
      roles: [],
      emptyReason: "还没有可读取的规范配置文件。",
      nextAction: "完成首次设置并创建 models.yml 与 config.yml。",
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overview) });

    await user.click(await screen.findByRole("link", { name: "完成首次设置" }));
    expect(await screen.findByRole("heading", { name: "设置 OMP" })).toBeVisible();
  });


  it("renders loading and error states without stale overview data, then retries", async () => {
    const user = userEvent.setup();
    let rejectInitial!: (error: unknown) => void;
    let resolveRetry!: (value: OverviewLoad) => void;
    const getOverviewLoad = vi.fn()
      .mockImplementationOnce(() => new Promise<OverviewLoad>((_, reject) => { rejectInitial = reject; }))
      .mockImplementationOnce(() => new Promise<OverviewLoad>((resolve) => { resolveRetry = resolve; }));
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad });

    expect(screen.getByRole("status", { name: "正在读取配置" })).toBeVisible();
    expect(screen.queryByText("1")).not.toBeInTheDocument();
    rejectInitial({ code: "overview-parse-error", message: "无法读取配置", action: "请在外部修复 YAML 后重新读取。" });
    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取配置");
    expect(document.querySelectorAll(".overview-skeleton")).toHaveLength(4);
    expect(screen.queryByText("1")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    expect(screen.getByRole("status", { name: "正在读取配置" })).toBeVisible();
    resolveRetry(overviewLoad(overviewDto()));
    expect((await screen.findAllByText("1"))[0]).toBeVisible();
  });
  it("offers the target directory action when overview loading fails", async () => {
    const user = userEvent.setup();
    const openTargetConfigurationDirectory = vi.fn(unavailableClient.openTargetConfigurationDirectory);
    const getOverviewLoad = async (): Promise<OverviewLoad> => ({
      startupState: readyState,
      overview: null,
      error: { code: "overview-read-failed", message: "无法读取配置", action: "请检查权限后重试。" },
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      openTargetConfigurationDirectory,
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取配置");
    await user.click(screen.getByRole("button", { name: "打开配置目录" }));
    expect(openTargetConfigurationDirectory).toHaveBeenCalledWith("/usr/local/bin/omp");
  });
  it("keeps startup metadata when overview load reports an error", async () => {
    const getOverviewLoad = async (): Promise<OverviewLoad> => ({
      startupState: readyState,
      overview: null,
      error: { code: "overview-read-failed", message: "无法读取配置", action: "请检查权限后重试。" },
    });
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad });

    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取配置");
    expect(screen.getByRole("link", { name: /OMP 已连接.*v17\.4\.1/ })).toBeVisible();
  });
  it("routes unconfirmed OMP switches to setup", async () => {
    const confirmationState: StartupState = {
      ...readyState,
      previousTargetConfiguration: "/Users/username/.omp/agent",
      requiresConfirmation: true,
    };
    const getOverviewLoad = async (): Promise<OverviewLoad> => ({
      startupState: confirmationState,
      overview: null,
      error: { code: "overview-confirmation-required", message: "无法读取尚未确认的 OMP 配置切换。", action: "请返回“设置 OMP”页确认新的 OMP 与 Target configuration 后再读取概览。" },
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getStartupState: async () => confirmationState,
      getOverviewLoad,
    });

    expect(await screen.findByRole("heading", { name: "确认切换 OMP" })).toBeVisible();
    expect(screen.getByRole("button", { name: "确认切换并进入应用" })).toBeVisible();
  });



  it("opens the Settings destination from the overview status footer", async () => {
    const user = userEvent.setup();
    renderRoute("/overview", { ...unavailableClient, getOverviewLoad: async () => overviewLoad(overviewDto()) });
    await screen.findByRole("heading", { name: "概览" });
    const footer = screen.getByRole("link", { name: /OMP 已连接/ });
    expect(footer).toHaveAttribute("href", "/settings#omp-settings");
    await user.click(footer);
    expect(await screen.findByRole("heading", { name: "设置" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "OMP 与 Target configuration" })).toBeVisible();
  });
});

describe("Model test page seam", () => {
  it("shows the one-time cost notice and starts a saved model test after confirmation", async () => {
    const user = userEvent.setup();
    const acceptModelTestCostNotice = vi.fn(async () => ({
      ompExecutablePath: "/usr/local/bin/omp",
      theme: "dark" as const,
      selectedProviderId: "dnslin",
      selectedModelId: "gpt-5.6-sol",
      modelTestCostNoticeAccepted: true,
    }));
    let latestResult: Awaited<ReturnType<TauriClient["testModel"]>> | null = null;
    const testModel = vi.fn(async () => {
      latestResult = {
        success: true,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 42,
        status: 200,
        message: "模型连接成功",
      };
      return latestResult;
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: false,
      }),
      acceptModelTestCostNotice,
      testModel,
      getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: latestResult, terminal: null }),
    });

    await screen.findByRole("region", { name: "快速测试" });
    await user.click(screen.getByRole("button", { name: "测试模型" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("模型测试会向 Provider 发起真实 API 请求，可能产生费用。");
    expect(testModel).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "继续测试" }));
    await waitFor(() => expect(acceptModelTestCostNotice).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(testModel).toHaveBeenCalledWith({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
    expect(await screen.findByText("模型连接成功")).toBeVisible();
    expect(screen.getByText("42 ms")).toBeVisible();
  });
  it("clears a completed result when refresh reconciles an invalidated remote state", async () => {
    const user = userEvent.setup();
    const refreshGate = deferred<void>();
    let loadCount = 0;
    const getOverviewLoad = vi.fn(async () => {
      loadCount += 1;
      if (loadCount > 1) await refreshGate.promise;
      return overviewLoad(overviewDto(), readyState);
    });
    const getModelTestState = vi.fn(async () => ({ running: false, providerId: null, modelId: null, result: null, terminal: null }));
    const testModel = vi.fn(async () => ({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 17,
      status: 200,
      message: "模型连接成功",
    }));
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    expect(await screen.findByText("模型连接成功")).toBeVisible();
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));
    expect(screen.getByText("模型连接成功")).toBeVisible();

    refreshGate.resolve();
    await waitFor(() => expect(getModelTestState.mock.calls.length).toBeGreaterThanOrEqual(2));
    const result = screen.getByRole("region", { name: "测试结果" });
    await waitFor(() => expect(result).toHaveTextContent("测试结果尚未测试模型"));
    expect(result).not.toHaveTextContent("模型连接成功");
  });
  it("clears a local result when the post-test overview refresh fails", async () => {
    const user = userEvent.setup();
    const refreshGate = deferred<void>();
    let loadCount = 0;
    let modelCompleted = false;
    const refreshError = {
      code: "overview-read-failed",
      message: "无法重新读取 models.yml。",
      action: "请检查文件后重试。",
    };
    const getOverviewLoad = vi.fn(async () => {
      loadCount += 1;
      if (loadCount > 1) {
        expect(modelCompleted).toBe(true);
        await refreshGate.promise;
        return { startupState: readyState, overview: null, error: refreshError };
      }
      return overviewLoad(overviewDto(), readyState);
    });
    const testModel = vi.fn(async () => {
      modelCompleted = true;
      return {
        success: true,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 17,
        status: 200,
        message: "模型连接成功",
      };
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    const initialLoads = getOverviewLoad.mock.calls.length;
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    expect(await screen.findByText("模型连接成功")).toBeVisible();
    refreshGate.resolve();
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(initialLoads + 1));

    const result = screen.getByRole("region", { name: "测试结果" });
    await waitFor(() => expect(result).toHaveTextContent("测试结果尚未测试模型"));
    expect(result).not.toHaveTextContent("模型连接成功");
  });


  it("exposes cancellation while a test is running and renders the cancelled result", async () => {
    const user = userEvent.setup();
    const pending = deferred<Awaited<ReturnType<TauriClient["testModel"]>>>();
    let latestResult: Awaited<ReturnType<TauriClient["testModel"]>> | null = null;
    const cancelModelTest = vi.fn(async () => {
      latestResult = {
        success: false,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 8,
        message: "测试已取消",
        errorCode: "cancelled",
      };
      pending.resolve(latestResult);
      return true;
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: latestResult, terminal: null }),
      testModel: () => pending.promise,
      cancelModelTest,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    expect(within(panel).getByRole("button", { name: "取消测试" })).toBeEnabled();
    await user.click(within(panel).getByRole("button", { name: "取消测试" }));
    await waitFor(() => expect(cancelModelTest).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("测试已取消")).toBeVisible();
  });
  it("reconciles a deferred preflight cancellation before showing the terminal state", async () => {
    const user = userEvent.setup();
    let rejected = false;
    let remotePolls = 0;
    const terminal = {
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      message: "测试已取消",
      errorCode: "cancelled",
    } as const;
    const getModelTestState = vi.fn(async () => {
      if (!rejected) return { running: false, providerId: null, modelId: null, result: null, terminal: null };
      remotePolls += 1;
      return remotePolls === 1
        ? { running: true, providerId: null, modelId: null, result: null, terminal }
        : { running: false, providerId: null, modelId: null, result: null, terminal };
    });
    const testModel = vi.fn(async () => {
      rejected = true;
      throw {
        code: "model-test-cancelled",
        message: "模型测试已取消。",
        action: "无需继续操作。",
      };
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    await waitFor(() => expect(remotePolls).toBeGreaterThanOrEqual(2), { timeout: 1000 });
    expect(await screen.findByText("测试已取消")).toBeVisible();
    expect(screen.getByRole("region", { name: "测试结果" })).not.toHaveTextContent("模型连接成功");
    expect(testModel).toHaveBeenCalledTimes(1);
  });

  it("refreshes after a backend busy conflict completes before remote running renders", async () => {
    const user = userEvent.setup();
    const remoteResult = {
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 24,
      status: 200,
      message: "远端测试成功",
    };
    const getModelTestState = vi.fn(async () => {
      const callCount = getModelTestState.mock.calls.length;
      if (callCount <= 2) return { running: false, providerId: null, modelId: null, result: null, terminal: null };
      return { running: false, providerId: null, modelId: null, result: remoteResult, terminal: null };
    });
    const testModel = vi.fn(async () => {
      throw { code: "model-test-busy", message: "已有模型测试正在进行。", action: "请等待当前测试完成。" };
    });
    const getOverviewLoad = vi.fn(async () => overviewLoad(overviewDto(), readyState));
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    await screen.findByRole("region", { name: "快速测试" });
    await waitFor(() => expect(getModelTestState.mock.calls.length).toBeGreaterThanOrEqual(2));
    await user.click(screen.getByRole("button", { name: "测试模型" }));
    expect(testModel).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("远端测试成功", {}, { timeout: 1000 })).toBeVisible();
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));
  });

  it("keeps the global cancel action in the result panel after changing the quick-test selection", async () => {
    const user = userEvent.setup();
    const pending = deferred<Awaited<ReturnType<TauriClient["testModel"]>>>();
    let latestResult: Awaited<ReturnType<TauriClient["testModel"]>> | null = null;
    const cancelModelTest = vi.fn(async () => {
      latestResult = {
        success: false,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 8,
        message: "测试已取消",
        errorCode: "cancelled",
      };
      pending.resolve(latestResult);
      return true;
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewSelectionDto(), readyState),
      getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: latestResult, terminal: null }),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel: () => pending.promise,
      cancelModelTest,
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    expect(within(panel).getByRole("button", { name: "取消测试" })).toBeEnabled();
    await user.click(screen.getByRole("combobox", { name: "Provider" }));
    await user.click(await screen.findByRole("option", { name: "anthropic" }));
    expect(within(panel).getByRole("button", { name: "测试模型" })).toBeDisabled();
    const result = screen.getByRole("region", { name: "测试结果" });
    const globalCancel = within(result).getByRole("button", { name: "取消测试" });
    expect(globalCancel).toBeEnabled();
    await user.click(globalCancel);
    await waitFor(() => expect(cancelModelTest).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("测试已取消")).toBeVisible();
  });

  it("provides a saved-model test action from Provider Detail without using form values", async () => {
    const user = userEvent.setup();
    const testModel = vi.fn(async () => ({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 16,
      status: 200,
      message: "模型连接成功",
    }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    const row = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "测试模型" }));
    await waitFor(() => expect(testModel).toHaveBeenCalledWith({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
  });

  it("waits for settings hydration before enabling Provider Detail model testing", async () => {
    const user = userEvent.setup();
    const settings = deferred<Awaited<ReturnType<TauriClient["getUiSettings"]>>>();
    const testModel = vi.fn(async () => ({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 16,
      status: 200,
      message: "模型连接成功",
    }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: () => settings.promise,
      testModel,
    });

    const row = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    const menuItem = screen.getByRole("menuitem", { name: "测试模型" });
    expect(menuItem).toBeDisabled();
    settings.resolve({
      ompExecutablePath: "/usr/local/bin/omp",
      theme: "dark",
      selectedProviderId: "dnslin",
      selectedModelId: "gpt-5.6-sol",
      modelTestCostNoticeAccepted: true,
    });
    await waitFor(() => expect(menuItem).toBeEnabled());
    await user.click(menuItem);
    await waitFor(() => expect(testModel).toHaveBeenCalledWith({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
    expect(screen.queryByRole("dialog", { name: "模型测试费用说明" })).not.toBeInTheDocument();
  });

  it("tests the saved model from the edit sheet instead of dirty form values", async () => {
    const user = userEvent.setup();
    const acceptModelTestCostNotice = vi.fn(unavailableClient.acceptModelTestCostNotice);
    const testModel = vi.fn(async () => ({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 18,
      status: 200,
      message: "模型连接成功",
    }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: false,
      }),
      acceptModelTestCostNotice,
      testModel,
    });

    const row = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const sheet = await screen.findByRole("dialog");
    await user.click(within(sheet).getByRole("button", { name: "测试模型" }));
    const dialog = await screen.findByRole("dialog", { name: "模型测试费用说明" });
    await user.click(within(dialog).getByRole("button", { name: "继续测试" }));
    await waitFor(() => expect(acceptModelTestCostNotice).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(testModel).toHaveBeenCalledWith({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
  });

  it("keeps dirty Model edits and their concurrency hash across a background refresh", async () => {
    const user = userEvent.setup();
    const initial = overviewDto();
    const refreshed = overviewDto({
      files: {
        ...initial.files,
        models: { ...initial.files.models, contentHash: "models-hash-after-refresh" },
      },
      providers: [{ ...initial.providers[0], baseUrl: "https://updated.example/v1" }],
    });
    let loadCount = 0;
    const getOverviewLoad = vi.fn(async () => overviewLoad(structuredClone(loadCount++ === 0 ? initial : refreshed), readyState));
    const testModel = vi.fn(async () => ({
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 18,
      status: 200,
      message: "模型连接成功",
    }));
    const editModel = vi.fn(async () => ({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
      editModel,
    });

    const row = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "编辑" }));
    const sheet = await screen.findByRole("dialog", { name: "编辑模型" });
    const name = within(sheet).getByRole("textbox", { name: "名称" });
    await user.clear(name);
    await user.type(name, "Dirty name");
    await user.click(within(sheet).getByRole("button", { name: "测试模型" }));

    await waitFor(() => expect(testModel).toHaveBeenCalledWith({ providerId: "dnslin", modelId: "gpt-5.6-sol" }));
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("dialog", { name: "编辑模型" })).toBeVisible();
    expect(within(screen.getByRole("dialog", { name: "编辑模型" })).getByRole("textbox", { name: "名称" })).toHaveValue("Dirty name");
    expect(within(screen.getByRole("dialog", { name: "编辑模型" })).getByRole("textbox", { name: "最终地址" })).toHaveValue("https://updated.example/v1/responses");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
    await user.click(within(screen.getByRole("dialog", { name: "编辑模型" })).getByRole("button", { name: "保存模型" }));
    await waitFor(() => expect(editModel).toHaveBeenCalledWith(expect.objectContaining({ openedModelsHash: "models-hash" })));
  });

  it("recovers a restored result after transient remote-state polling failures", async () => {
    const restoredResult = {
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 21,
      status: 200,
      message: "恢复后的模型连接成功",
    };
    const getModelTestState = vi.fn(async () => {
      if (getModelTestState.mock.calls.length <= 2) {
        throw { code: "state-read-failed", message: "暂时无法读取模型测试状态。", action: "请稍后重试。" };
      }
      return { running: false, providerId: null, modelId: null, result: restoredResult, terminal: null };
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    await screen.findByRole("region", { name: "快速测试" });
    expect(await screen.findByText("恢复后的模型连接成功", {}, { timeout: 1500 })).toBeVisible();
    expect(getModelTestState.mock.calls.length).toBeGreaterThan(2);
  });

  it.each(["/overview", "/providers/dnslin"] as const)("refreshes %s once after remote polling finishes", async (route) => {
    const runningResult = {
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 12,
      status: 200,
      message: "旧测试结果",
    };
    const completedResult = { ...runningResult, latencyMs: 19, message: "模型连接成功" };
    const getModelTestState = vi.fn(async () => {
      if (getModelTestState.mock.calls.length < 4) {
        return { running: true, providerId: "dnslin", modelId: "gpt-5.6-sol", result: structuredClone(runningResult), terminal: null };
      }
      return { running: false, providerId: null, modelId: null, result: structuredClone(completedResult), terminal: null };
    });
    const getOverviewLoad = vi.fn(async () => overviewLoad(structuredClone(overviewDto()), readyState));
    renderRoute(route, {
      ...unavailableClient,
      getOverviewLoad,
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    await screen.findByRole(route === "/overview" ? "region" : "heading", route === "/overview" ? { name: "快速测试" } : { name: "dnslin" });
    await waitFor(() => expect(getModelTestState.mock.calls.length).toBeGreaterThanOrEqual(4), { timeout: 2000 });
    expect(await screen.findByText("模型连接成功")).toBeVisible();
    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(2), { timeout: 2000 });
    await waitFor(() => expect(getModelTestState.mock.calls.length).toBeGreaterThanOrEqual(5), { timeout: 2000 });
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("does not replace a delayed initial overview load before reconciling a restored result", async () => {
    const initialLoad = deferred<OverviewLoad>();
    const restoredResult = {
      success: true,
      providerId: "dnslin",
      modelId: "gpt-5.6-sol",
      protocol: "openai-responses" as const,
      latencyMs: 18,
      status: 200,
      message: "模型连接成功",
    };
    const overview = overviewDto();
    let loadCount = 0;
    const getOverviewLoad = vi.fn(async () => {
      if (loadCount++ === 0) return initialLoad.promise;
      return overviewLoad(structuredClone(overview), readyState);
    });
    const getModelTestState = vi.fn(async () => ({ running: false, providerId: null, modelId: null, result: restoredResult, terminal: null }));
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad,
      getModelTestState,
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    expect(screen.getByRole("status", { name: "正在读取配置" })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(1);
    initialLoad.resolve(overviewLoad(overview, readyState));
    await screen.findByRole("region", { name: "快速测试" });
    expect(screen.getByText("模型连接成功")).toBeVisible();
    await waitFor(() => expect(getModelTestState.mock.calls.length).toBeGreaterThanOrEqual(2));
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getOverviewLoad).toHaveBeenCalledTimes(2);
  });

  it("disables saved-model testing when API key mode has no saved key", async () => {
    const base = overviewDto();
    const provider = { ...base.providers[0], hasApiKey: false };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider] }), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(within(panel).getByRole("button", { name: "测试模型" })).toBeDisabled();
  });

  it("disables saved-model testing for a read-only Target configuration", async () => {
    const base = overviewDto();
    const readOnlyTarget = { ...base.targetConfiguration, status: "read-only" as const, writable: false };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ targetConfiguration: readOnlyTarget }), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(within(panel).getByRole("button", { name: "测试模型" })).toBeDisabled();
  });

  it("disables saved-model testing when Max Tokens exceeds Context Window", async () => {
    const base = overviewDto();
    const incompleteModel = { ...base.models[0], contextWindow: 1000, maxTokens: 2000, complete: false, status: "incomplete" as const };
    const provider = { ...base.providers[0], models: [incompleteModel] };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider], models: [incompleteModel] }), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(within(panel).getByRole("button", { name: "测试模型" })).toBeDisabled();
  });

  it("disables saved-model testing for a complete image-only model", async () => {
    const base = overviewDto();
    const imageOnlyModel: OverviewModel = {
      ...base.models[0],
      input: ["image"] as OverviewModel["input"],
      complete: true,
      status: "normal",
    };
    const provider = { ...base.providers[0], models: [imageOnlyModel] };
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto({ providers: [provider], models: [imageOnlyModel] }), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    expect(within(panel).getByRole("button", { name: "测试模型" })).toBeDisabled();
  });

  it("does not expose testing for an unsaved copy sheet", async () => {
    const user = userEvent.setup();
    const testModel = vi.fn(unavailableClient.testModel);
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
    });

    const row = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "复制" }));
    const sheet = await screen.findByRole("dialog");
    expect(within(sheet).getByRole("button", { name: "测试模型" })).toBeDisabled();
    expect(testModel).not.toHaveBeenCalled();
  });

  it("shows visible progress and disables other saved-model test actions while one is running", async () => {
    const user = userEvent.setup();
    const pending = deferred<Awaited<ReturnType<TauriClient["testModel"]>>>();
    let latestResult: Awaited<ReturnType<TauriClient["testModel"]>> | null = null;
    const cancelModelTest = vi.fn(async () => {
      const cancelledResult = {
        success: false,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 5,
        message: "测试已取消",
        errorCode: "cancelled",
      };
      latestResult = cancelledResult;
      pending.resolve(cancelledResult);
      return true;
    });
    const base = overviewDto();
    const secondModel = { ...base.models[0], id: "gpt-5.6-terra", name: "Terra" };
    const overview = overviewDto({
      counts: { ...base.counts, modelCount: 2 },
      providers: [{ ...base.providers[0], modelCount: 2, models: [base.models[0], secondModel] }],
      models: [base.models[0], secondModel],
    });
    renderRoute("/providers/dnslin", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overview, readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel: () => pending.promise,
      getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: latestResult, terminal: null }),
      cancelModelTest,
    });

    const firstRow = (await screen.findByText("gpt-5.6-sol", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(firstRow).getByRole("button", { name: "Model 操作 gpt-5.6-sol" }));
    await user.click(screen.getByRole("menuitem", { name: "测试模型" }));
    expect(within(firstRow).getByText("测试中…", { exact: true })).toBeVisible();
    expect(screen.getByRole("button", { name: "取消测试 dnslin/gpt-5.6-sol" })).toBeEnabled();

    const secondRow = (await screen.findByText("gpt-5.6-terra", { exact: true })).closest("tr") as HTMLElement;
    await user.click(within(secondRow).getByRole("button", { name: "Model 操作 gpt-5.6-terra" }));
    expect(screen.getByRole("menuitem", { name: "测试模型" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "取消测试 dnslin/gpt-5.6-sol" }));
    await waitFor(() => expect(cancelModelTest).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByRole("cell", { name: "测试已取消" })).toBeVisible());
  });

  it("renders a failed result with the shared danger status", async () => {
    const user = userEvent.setup();
    let latestResult: Awaited<ReturnType<TauriClient["testModel"]>> | null = null;
    const testModel = vi.fn(async () => {
      const failure = {
        success: false,
        providerId: "dnslin",
        modelId: "gpt-5.6-sol",
        protocol: "openai-responses" as const,
        latencyMs: 21,
        status: 401,
        message: "Provider 拒绝了认证，请检查 API Key。",
        errorCode: "http-401",
      };
      latestResult = failure;
      return failure;
    });
    renderRoute("/overview", {
      ...unavailableClient,
      getOverviewLoad: async () => overviewLoad(overviewDto(), readyState),
      getUiSettings: async () => ({
        ompExecutablePath: "/usr/local/bin/omp",
        theme: "dark",
        selectedProviderId: "dnslin",
        selectedModelId: "gpt-5.6-sol",
        modelTestCostNoticeAccepted: true,
      }),
      testModel,
      getModelTestState: async () => ({ running: false, providerId: null, modelId: null, result: latestResult, terminal: null }),
    });

    const panel = await screen.findByRole("region", { name: "快速测试" });
    await user.click(within(panel).getByRole("button", { name: "测试模型" }));
    const result = await screen.findByRole("region", { name: "测试结果" });
    await waitFor(() => expect(result).toHaveTextContent("Provider 拒绝了认证，请检查 API Key。"));
    expect(result.querySelector(".status-indicator--danger")).not.toBeNull();
  });
});
