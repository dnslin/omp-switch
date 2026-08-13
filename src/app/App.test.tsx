import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import { App } from "./App";
import { TauriClientProvider, type TauriClient } from "../lib/tauri-client";

const unavailableClient: TauriClient = {
  getStartupState: async () => ({
    kind: "omp-unavailable",
    message: "尚未检测 OMP",
  }),
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
  it("renders the real setup route with the Rust-produced unavailable state", async () => {
    renderRoute("/setup");

    expect(await screen.findByRole("heading", { name: "设置 OMP" })).toBeVisible();
    expect(screen.getByText("尚未检测 OMP")).toBeVisible();
    expect(screen.getByRole("button", { name: "自动检测" })).toBeDisabled();
    expect(screen.getByText("OMP 检测将在后续工单中启用。")).toBeVisible();
  });

  it("redirects the root route and marks overview active", async () => {
    renderRoute("/");

    expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
    expect(screen.getByRole("link", { name: "概览" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("banner", { name: "应用状态" })).toBeVisible();
  });

  it("navigates the main shell through accessible links", async () => {
    const user = userEvent.setup();
    renderRoute("/overview");

    await user.click(screen.getByRole("link", { name: "Providers" }));
    expect(screen.getByRole("heading", { name: "Providers" })).toBeVisible();
    expect(screen.getByText("Provider 管理将在后续工单中实现。")).toBeVisible();
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
