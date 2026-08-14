import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { TauriClientProvider, type StartupState, type TargetConfigurationDiscovery, type TauriClient } from "../lib/tauri-client";


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
    createPaths: [],
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


function renderRoute(route: string, client: TauriClient = unavailableClient) {
  return render(
    <TauriClientProvider client={client}>
      <MemoryRouter initialEntries={[route]}>
        <App />
      </MemoryRouter>
    </TauriClientProvider>,
  );
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
    expect(initializeTargetConfiguration).toHaveBeenCalledWith("/usr/local/bin/omp", creationState.targetConfiguration.createPaths);
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

    expect(initializeTargetConfiguration).toHaveBeenCalledWith("/usr/local/bin/omp", creationState.targetConfiguration.createPaths);
    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
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

  it("navigates the main shell through accessible links", async () => {
    const user = userEvent.setup();
    renderRoute("/overview");

    await user.click(screen.getByRole("link", { name: "Providers" }));
    expect(screen.getByRole("heading", { name: "Providers" })).toBeVisible();
    expect(screen.getByText("Provider 管理将在后续工单中实现。")).toBeVisible();
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

  it("shows the AppError message and next action without exposing unrelated details", async () => {
    renderRoute("/overview", {
      ...unavailableClient,
      getUiSettings: async () => {
        throw {
          code: "settings-read-failed",
          message: "无法读取界面状态",
          action: "请检查应用数据目录权限。",
          internal: "sensitive internal detail",
        };
      },
    });

    expect(await screen.findByText("无法读取界面状态")).toBeVisible();
    expect(screen.getByText("请检查应用数据目录权限。")).toBeVisible();
    expect(screen.queryByText("sensitive internal detail")).not.toBeInTheDocument();
  });
});
