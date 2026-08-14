import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { TauriClientProvider, type StartupState, type TauriClient } from "../lib/tauri-client";

const unavailableClient: TauriClient = {
  getStartupState: async () => ({
    kind: "omp-unavailable",
    message: "未在已保存路径或系统 PATH 中找到可用的 OMP。",
  }),
  detectOmp: async () => ({ kind: "omp-unavailable", message: "仍未找到 OMP" }),
  selectOmpExecutable: async () => null,
  validateSelectedOmp: async () => ({ kind: "invalid-executable", executablePath: "/tmp/not-omp", message: "无法运行", diagnosticCode: "io-not-found" }),
  confirmSelectedOmp: async () => undefined,
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
  targetConfiguration: "/Users/username/.omp/agent",
  targetAccess: { writable: true, modelsYml: "normal", configYml: "normal" },
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
        targetConfiguration: "/Users/username/.omp/agent",
        targetAccess: { writable: true, modelsYml: "normal", configYml: "normal" },
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
      targetConfiguration: "/Users/username/.omp/new-agent",
      previousTargetConfiguration: readyState.targetConfiguration,
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
    expect(enterButton).toHaveClass("disabled:opacity-100");

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

  it("keeps a configuration-disabled enter button muted during redetection", async () => {
    const user = userEvent.setup();
    let resolveDetection!: (state: StartupState) => void;
    const missingConfigurationState: StartupState = {
      ...readyState,
      targetAccess: { writable: true, modelsYml: "missing", configYml: "normal" },
    };
    const detectOmp = vi.fn(() => new Promise<StartupState>((resolve) => { resolveDetection = resolve; }));
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => missingConfigurationState,
      detectOmp,
    });

    expect(await screen.findByText("缺失")).toBeVisible();
    const enterButton = screen.getByRole("button", { name: "进入应用" });
    expect(enterButton).toBeDisabled();
    expect(enterButton).toHaveClass("disabled:opacity-50");

    await user.click(screen.getByRole("button", { name: "重新检测" }));
    expect(enterButton).toBeDisabled();
    expect(enterButton).toHaveClass("disabled:opacity-50");
    expect(enterButton).not.toHaveClass("disabled:opacity-100");

    resolveDetection(missingConfigurationState);
    await waitFor(() => expect(screen.getByRole("button", { name: "重新检测" })).toBeEnabled(), { timeout: 2000 });
    expect(enterButton).toHaveClass("disabled:opacity-50");
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
        targetConfiguration: "/Users/username/.omp/new-agent",
        previousTargetConfiguration: "/Users/username/.omp/old-agent",
        targetAccess: { writable: true, modelsYml: "normal", configYml: "normal" },
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
