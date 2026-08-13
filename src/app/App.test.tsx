import { render, screen } from "@testing-library/react";
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
  validateSelectedOmp: async () => ({ kind: "invalid-executable", executablePath: "/tmp/not-omp", message: "无法运行" }),
  confirmSelectedOmp: async () => undefined,
  getUiSettings: async () => ({
    ompExecutablePath: null,
    theme: "system",
    selectedProviderId: null,
    selectedModelId: null,
    costNoticeAccepted: false,
  }),
  saveUiSettings: async (settings) => settings,
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
    [{ kind: "invalid-executable", executablePath: "/tmp/not-omp", message: "所选文件无法运行" } as const, "所选文件无法运行"],
    [{ kind: "version-failed", executablePath: "/tmp/omp", message: "版本失败", exitCode: 7, stderr: "技术详情已脱敏" } as const, "版本失败"],
    [{ kind: "config-path-failed", executablePath: "/tmp/omp", version: "17.4.1", message: "不会猜测目录。该命令可能初始化 OMP Settings、访问 agent.db，或运行 OMP 自身的旧迁移。", exitCode: 9, stderr: "技术详情已脱敏" } as const, "不会猜测目录"],
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

  it("keeps the successful setup layout mounted while redetection is pending", async () => {
    const user = userEvent.setup();
    let resolveDetection!: (state: StartupState) => void;
    const detectOmp = vi.fn(() => new Promise<StartupState>((resolve) => { resolveDetection = resolve; }));
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => ({
        kind: "omp-ready",
        executablePath: "/usr/local/bin/omp",
        version: "17.4.1",
        targetConfiguration: "/Users/username/.omp/agent",
        targetAccess: { writable: true, modelsYml: "normal", configYml: "normal" },
        requiresConfirmation: false,
      }),
      detectOmp,
    });

    expect(await screen.findByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "重新检测" }));

    expect(screen.getByRole("heading", { name: "OMP 已找到" })).toBeVisible();
    expect(screen.getByText("/Users/username/.omp/agent")).toBeVisible();
    expect(screen.getByRole("button", { name: "正在重新检测" })).toBeDisabled();

    resolveDetection({ kind: "omp-unavailable", message: "仍未找到 OMP" });
    expect((await screen.findAllByText("仍未找到 OMP"))[0]).toBeVisible();
  });

  it("shows missing files without allowing entry or initializing them", async () => {
    renderRoute("/setup", {
      ...unavailableClient,
      getStartupState: async () => ({
        kind: "omp-ready",
        executablePath: "/usr/local/bin/omp",
        version: "17.4.1",
        targetConfiguration: "/Users/username/.omp/agent",
        targetAccess: { writable: true, modelsYml: "missing", configYml: "normal" },
        requiresConfirmation: false,
      }),
    });

    expect(await screen.findByText("缺失")).toBeVisible();
    expect(screen.getByRole("button", { name: "进入应用" })).toBeDisabled();
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

  it("fills the application viewport with a bounded sidebar and content region", () => {
    renderRoute("/overview");

    const shell = screen.getByRole("main");
    expect(shell).toHaveClass("shell-main");
    expect(shell.parentElement).toHaveClass("app-frame", "app-frame--shell");
    expect(screen.getByRole("navigation", { name: "主导航" }).closest("aside")).toHaveClass("sidebar");
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
