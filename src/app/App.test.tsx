import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { TauriClientProvider, type OverviewDto, type OverviewLoad, type OverviewModel, type OverviewProvider, type StartupState, type TargetConfigurationDiscovery, type TauriClient } from "../lib/tauri-client";


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
    costNoticeAccepted: false,
  }),
  saveUiSettings: async (settings) => ({ ompExecutablePath: null, ...settings }),
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
  const app = (
    <TauriClientProvider client={client}>
      <MemoryRouter initialEntries={[route]}>
        <App />
      </MemoryRouter>
    </TauriClientProvider>
  );
  return render(strictMode ? <StrictMode>{app}</StrictMode> : app);
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

  it("lists searchable Provider safety summaries and freezes unsafe actions", async () => {
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
    expect(screen.getByRole("button", { name: "OpenAI 操作" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "advanced 操作" })).toBeDisabled();

    await user.type(screen.getByRole("searchbox", { name: "搜索 Provider" }), "claude");
    expect(screen.queryByRole("link", { name: "OpenAI" })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "advanced" })).toBeVisible();

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
    expect(await screen.findByRole("link", { name: /OMP 不可用.*配置目录不可用/ })).toBeVisible();
    expect(getOverviewLoad).toHaveBeenCalledTimes(1);
    expect(getStartupState).toHaveBeenCalledTimes(1);
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
  it("ignores stale Providers data after navigation", async () => {
    const user = userEvent.setup();
    const first = deferred<OverviewLoad>();
    const getOverviewLoad = vi.fn(() => first.promise);
    const getStartupState = vi.fn(async () => ({ kind: "omp-unavailable", message: "最新 OMP 状态不可用" } as const));
    renderRoute("/providers", { ...unavailableClient, getOverviewLoad, getStartupState });

    await waitFor(() => expect(getOverviewLoad).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("link", { name: "角色" }));
    expect(await screen.findByRole("heading", { name: "角色" })).toBeVisible();
    expect(await screen.findByRole("link", { name: /最新 OMP 状态不可用/ })).toBeVisible();
    first.resolve(overviewLoad(overviewDto(), readyState));
    await waitFor(() => expect(screen.getByRole("link", { name: /最新 OMP 状态不可用/ })).toBeVisible());
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
    editable: true,
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
    providers: [{ id: "dnslin", name: "Local", baseUrl: "https://example.com", defaultApi: "openai-responses", authMode: "api-key", hasApiKey: true, modelCount: 1, classification: "custom", editable: true, readOnlyReason: null, models: [model] }],
    models: [model],
    roles: [{ id: "default", status: "configured", selector: "dnslin/gpt-5.6-sol:max" }, { id: "task", status: "configured", selector: "dnslin/gpt-5.6-sol" }],
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
    expect(screen.getByRole("button", { name: "测试模型（尚未启用）" })).toBeDisabled();
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
        costNoticeAccepted: true,
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
      costNoticeAccepted: true,
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
      costNoticeAccepted: true,
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
        costNoticeAccepted: true,
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
      costNoticeAccepted: true,
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
        costNoticeAccepted: true,
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
      costNoticeAccepted: true,
    });
    secondSave.resolve({
      ompExecutablePath: null,
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
      costNoticeAccepted: true,
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
        costNoticeAccepted: true,
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

    firstSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "anthropic", selectedModelId: null, costNoticeAccepted: true });
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(2));
    expect(saveUiSettings).toHaveBeenNthCalledWith(2, {
      theme: "dark",
      selectedProviderId: "anthropic",
      selectedModelId: "claude-sonnet-4",
      costNoticeAccepted: true,
    });
    secondSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "anthropic", selectedModelId: "claude-sonnet-4", costNoticeAccepted: true });
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(3));
    expect(saveUiSettings).toHaveBeenNthCalledWith(3, {
      theme: "dark",
      selectedProviderId: "dnslin",
      selectedModelId: null,
      costNoticeAccepted: true,
    });
    thirdSave.resolve({ ompExecutablePath: null, theme: "dark", selectedProviderId: "dnslin", selectedModelId: null, costNoticeAccepted: true });
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
      costNoticeAccepted: false,
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
        costNoticeAccepted: true,
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
        costNoticeAccepted: false,
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
        costNoticeAccepted: true,
      }),
      saveUiSettings,
    }, true);

    expect(await screen.findByText("之前选择的模型已不存在，请重新选择。")).toBeVisible();
    await waitFor(() => expect(saveUiSettings).toHaveBeenCalledTimes(1));
    expect(saveUiSettings).toHaveBeenCalledWith({
      theme: "dark",
      selectedProviderId: expectedProviderId,
      selectedModelId: expectedModelId,
      costNoticeAccepted: true,
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
    expect(screen.getByRole("button", { name: "测试模型（尚未启用）" })).toBeDisabled();
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
