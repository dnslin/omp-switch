import { LayoutGrid, Server, Settings, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes } from "react-router";
import { toast, Toaster } from "sonner";
import { Button, Card, NavigationItem, PageTitle, StatusIndicator } from "../components/ui";
import { asAppError, useTauriClient, type StartupState } from "../lib/tauri-client";

const pages = [
  { to: "/overview", label: "概览", icon: LayoutGrid },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/roles", label: "角色", icon: Users },
  { to: "/settings", label: "设置", icon: Settings },
] as const;

function ApplicationHeader() {
  return <header className="application-header" aria-label="应用状态" />;
}

function MainShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="app-frame">
      <ApplicationHeader />
      <main className="shell-main">
        <aside className="sidebar">
          <nav className="sidebar-nav" aria-label="主导航">
            {pages.map(({ to, label, icon }) => (
              <NavLink key={to} to={to}>
                {({ isActive }) => <NavigationItem active={isActive} icon={icon}>{label}</NavigationItem>}
              </NavLink>
            ))}
          </nav>
          <Link className="sidebar-footer" to="/settings">
            <strong>尚未检测 OMP</strong>
            <code>配置目录不可用</code>
          </Link>
        </aside>
        <section className="page-content">{children}</section>
      </main>
    </div>
  );
}

function SetupPage() {
  const client = useTauriClient();
  const [state, setState] = useState<StartupState | null>(null);

  useEffect(() => {
    let active = true;
    void client.getStartupState().then((next) => {
      if (active) setState(next);
    }).catch((error: unknown) => {
      const appError = asAppError(error, "无法读取启动状态");
      toast.error(appError.message, { description: appError.action });
    });
    return () => { active = false; };
  }, [client]);

  return (
    <div className="app-frame">
      <ApplicationHeader />
      <main className="setup-body">
        <section className="setup-card">
          <header>
            <h1>设置 OMP</h1>
            <p>连接 OMP 后，OMP Switch 才能读取权威配置目录。</p>
          </header>
          <div className="setup-state" aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {state?.message ?? "正在读取应用状态…"}
          </div>
          <div className="setup-table">
            <div className="setup-row"><span>可执行文件</span><code>尚未选择</code><span>未检测</span></div>
            <div className="setup-row"><span>版本</span><span>—</span><span>未知</span></div>
            <div className="setup-row"><span>权威配置目录</span><code>等待 omp config path</code><span>不可用</span></div>
            <div className="setup-row"><span>models.yml</span><span>—</span><span>未读取</span></div>
            <div className="setup-row"><span>config.yml</span><span>—</span><span>未读取</span></div>
          </div>
          <p className="setup-unavailable-note">OMP 检测将在后续工单中启用。</p>
          <div className="setup-actions">
            <Button variant="secondary" disabled>手动选择 OMP</Button>
            <Button disabled>自动检测</Button>
          </div>
        </section>
      </main>
    </div>
  );
}

const routeCopy = {
  overview: ["概览", "查看当前配置状态并快速验证模型连接。", "尚未检测 OMP。完成首次设置后，这里会显示权威配置状态。"],
  providers: ["Providers", "管理自定义 Provider 与模型。", "Provider 管理将在后续工单中实现。"],
  roles: ["角色", "管理 OMP 模型角色。", "角色管理将在后续工单中实现。"],
  settings: ["设置", "配置 OMP 路径、主题与轻量界面偏好。", "设置能力将在后续工单中扩展；当前不会保存任何 Provider、Model definition、Model role 或 Direct API Key。"],
} as const;

function PlaceholderPage({ page }: { page: keyof typeof routeCopy }) {
  const [title, description, message] = routeCopy[page];
  return (
    <MainShell>
      <PageTitle title={title} description={description} />
      <Card title="当前状态">
        <StatusIndicator tone="neutral">尚未检测 OMP</StatusIndicator>
        <p className="placeholder-card">{message}</p>
      </Card>
    </MainShell>
  );
}

function ProviderDetailPage() {
  return (
    <MainShell>
      <PageTitle title="Provider 详情" description="查看 Provider 与其模型。" />
      <Card title="未载入 Provider"><p className="placeholder-card">Provider 详情将在后续工单中实现，不显示伪造数据。</p></Card>
    </MainShell>
  );
}

function NotFoundPage() {
  return (
    <main className="not-found">
      <h1>页面不存在</h1>
      <p>该地址不属于 OMP Switch。</p>
      <Button asChild><Link to="/overview">返回概览</Link></Button>
    </main>
  );
}

export function App() {
  const client = useTauriClient();

  useEffect(() => {
    void client.getUiSettings().catch((error: unknown) => {
      const appError = asAppError(error, "无法读取界面状态");
      toast.error(appError.message, { description: appError.action });
    });
  }, [client]);

  return (
    <div className="window">
      <Routes>
        <Route path="/" element={<Navigate replace to="/overview" />} />
        <Route path="/setup" element={<SetupPage />} />
        <Route path="/overview" element={<PlaceholderPage page="overview" />} />
        <Route path="/providers" element={<PlaceholderPage page="providers" />} />
        <Route path="/providers/:providerId" element={<ProviderDetailPage />} />
        <Route path="/roles" element={<PlaceholderPage page="roles" />} />
        <Route path="/settings" element={<PlaceholderPage page="settings" />} />
        <Route path="*" element={<NotFoundPage />} />
      </Routes>
      <Toaster position="bottom-right" richColors />
    </div>
  );
}
