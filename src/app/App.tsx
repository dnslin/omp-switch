import { CheckCircle2, File, Folder, Info, LayoutGrid, Server, Settings, SquareTerminal, Users } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useNavigate } from "react-router";
import { toast, Toaster } from "sonner";
import { RedetectionLoader } from "../components/redetection-loader";
import { Button, Card, NavigationItem, PageTitle, StatusIndicator } from "../components/ui";
import { asAppError, useTauriClient, type StartupState } from "../lib/tauri-client";

const pages = [
  { to: "/overview", label: "概览", icon: LayoutGrid },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/roles", label: "角色", icon: Users },
  { to: "/settings", label: "设置", icon: Settings },
] as const;
const REDETECT_MINIMUM_DURATION_MS = 1200;



function MainShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="app-frame app-frame--shell">
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

type ReadyState = Extract<StartupState, { kind: "omp-ready" }>;
type FailureState = Extract<StartupState, { kind: "invalid-executable" | "version-failed" | "config-path-failed" }>;

type SetupPresentation = {
  readyState: ReadyState | null;
  failureState: FailureState | null;
  confirmingSwitch: boolean;
  title: string;
  description: string;
  statusText: string;
};

function getSetupPresentation(state: StartupState): SetupPresentation {
  switch (state.kind) {
    case "detecting":
      return {
        readyState: null,
        failureState: null,
        confirmingSwitch: false,
        title: "正在检测 OMP…",
        description: "正在检查可执行文件、版本和权威配置目录。",
        statusText: "正在检测 OMP…",
      };
    case "omp-unavailable":
      return {
        readyState: null,
        failureState: null,
        confirmingSwitch: false,
        title: "设置 OMP",
        description: state.message,
        statusText: state.message,
      };
    case "invalid-executable":
    case "version-failed":
      return {
        readyState: null,
        failureState: state,
        confirmingSwitch: false,
        title: "设置 OMP",
        description: state.message,
        statusText: state.message,
      };
    case "config-path-failed":
      return {
        readyState: null,
        failureState: state,
        confirmingSwitch: false,
        title: "无法获取 OMP 配置目录",
        description: state.message,
        statusText: state.message,
      };
    case "omp-ready": {
      const confirmingSwitch = state.requiresConfirmation;
      return {
        readyState: state,
        failureState: null,
        confirmingSwitch,
        title: confirmingSwitch ? "确认切换 OMP" : "OMP 已找到",
        description: confirmingSwitch
          ? "请确认新的 OMP 及其 Target configuration；确认后才会替换当前选择。"
          : "OMP Switch 已确认可执行文件和权威配置目录。",
        statusText: confirmingSwitch ? "等待确认  ·  尚未切换 OMP" : "检测完成  ·  OMP 已可用",
      };
    }
  }
}


function SetupPage() {
  const client = useTauriClient();
  const navigate = useNavigate();
  const [state, setState] = useState<StartupState>({ kind: "detecting" });
  const [redetecting, setRedetecting] = useState(false);
  const detectionInFlight = useRef(false);
  useEffect(() => {
    let active = true;
    void client.getStartupState().then((next) => {
      if (active) setState(next);
    }).catch((error: unknown) => {
      const appError = asAppError(error, "无法读取启动状态");
      toast.error(appError.message, { description: appError.action });
    });
    return () => {
      active = false;
    };
  }, [client]);

  async function detect() {
    if (detectionInFlight.current) return;
    detectionInFlight.current = true;
    const preserveReadyState = state.kind === "omp-ready";
    if (preserveReadyState) {
      setRedetecting(true);
    } else {
      setState({ kind: "detecting" });
    }
    try {
      let nextState: StartupState;
      if (preserveReadyState) {
        const [detectionResult] = await Promise.allSettled([
          client.detectOmp(),
          new Promise<void>((resolve) => setTimeout(resolve, REDETECT_MINIMUM_DURATION_MS)),
        ]);
        if (detectionResult.status === "rejected") throw detectionResult.reason;
        nextState = detectionResult.value;
      } else {
        nextState = await client.detectOmp();
      }
      setState(nextState);
    } catch (error: unknown) {
      const appError = asAppError(error, "无法重新检测 OMP");
      toast.error(appError.message, { description: appError.action });
      setState({ kind: "omp-unavailable", message: appError.message });
    } finally {
      detectionInFlight.current = false;
      setRedetecting(false);
    }
  }

  async function selectExecutable() {
    try {
      const path = await client.selectOmpExecutable();
      if (!path) return;
      setState({ kind: "detecting" });
      setState(await client.validateSelectedOmp(path));
    } catch (error: unknown) {
      const appError = asAppError(error, "无法验证所选 OMP");
      toast.error(appError.message, { description: appError.action });
      setState({ kind: "omp-unavailable", message: appError.message });
    }
  }

  async function enterApplication() {
    if (state.kind !== "omp-ready") return;
    try {
      if (state.requiresConfirmation) {
        await client.confirmSelectedOmp(state.executablePath);
      }
      navigate("/overview");
    } catch (error: unknown) {
      const appError = asAppError(error, "无法保存 OMP 选择");
      toast.error(appError.message, { description: appError.action });
    }
  }
  const presentation = getSetupPresentation(state);
  const { readyState, failureState, confirmingSwitch, title, description, statusText } = presentation;
  const configurationReady = readyState !== null
    && readyState.targetAccess.modelsYml !== "missing"
    && readyState.targetAccess.configYml !== "missing";

  return (
    <div className="app-frame">
      <main className="setup-body">
        <section className="setup-card" aria-busy={redetecting}>
          <header><h1>{title}</h1><p>{description}</p></header>
          <div className={`setup-state ${readyState ? "setup-state--success" : ""}`} aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {statusText}
          </div>
          {readyState ? (
            <>
              <div className="setup-table">
                <SetupRow icon={SquareTerminal} label="可执行文件" value={readyState.executablePath} mono />
                <SetupRow icon={Info} label="版本" value={readyState.version} />
                <SetupRow icon={Folder} label="权威配置目录" value={readyState.targetConfiguration} mono status={readyState.targetAccess.writable ? "正常" : "只读"} />
                <SetupRow icon={File} label="models.yml" value="" status={fileStatusLabel(readyState.targetAccess.modelsYml)} />
                <SetupRow icon={File} label="config.yml" value="" status={fileStatusLabel(readyState.targetAccess.configYml)} />
              </div>
              {confirmingSwitch ? (
                <div className="target-change" aria-label="Target configuration 变更">
                  <div><span>当前 Target configuration</span><code>{readyState.previousTargetConfiguration ?? "当前 OMP 无法读取，配置目录未知"}</code></div>
                  <div><span>切换后 Target configuration</span><code>{readyState.targetConfiguration}</code></div>
                </div>
              ) : null}
              <div className="permission-summary"><CheckCircle2 aria-hidden="true" />{readyState.targetAccess.writable && readyState.targetAccess.modelsYml === "normal" && readyState.targetAccess.configYml === "normal" ? "配置文件可读写，权限正常。" : "配置目录已确认；只读或缺失文件状态如上。"}</div>
            </>
          ) : failureState ? (
            <details className="technical-details"><summary>查看技术详情</summary><p>诊断代码：{failureState.diagnosticCode}</p>{failureState.kind !== "invalid-executable" ? <><p>退出码：{failureState.exitCode ?? "不可用"}</p><p>{failureState.stderr || "命令没有返回 stderr。"}</p></> : null}</details>
          ) : null}
          <div className="setup-actions">
            <Button size="setup" variant="secondary" onClick={selectExecutable} disabled={state.kind === "detecting" || redetecting}>
              手动选择 OMP
            </Button>
            {readyState ? (
              <Button size="setup" variant="secondary" onClick={detect} disabled={redetecting} disabledAppearance="stable">
                重新检测
              </Button>
            ) : (
              <Button size="setup" onClick={detect} disabled={state.kind === "detecting"}>自动检测</Button>
            )}
            {readyState ? (
              <Button size="setup" onClick={enterApplication} disabled={!configurationReady || redetecting} disabledAppearance={configurationReady && redetecting ? "stable" : "muted"}>
                {confirmingSwitch ? "确认切换并进入应用" : "进入应用"}
              </Button>
            ) : null}
          </div>
          {redetecting ? (
            <div className="redetect-overlay" role="status" aria-live="polite" aria-label="正在重新检测 OMP" data-testid="redetect-progress">
              <div className="redetect-overlay__content">
                <RedetectionLoader />
                <strong>正在重新检测 OMP</strong>
              </div>
            </div>
          ) : null}
        </section>
      </main>
    </div>
  );
}

function SetupRow({ icon: Icon, label, value, mono = false, status = "正常" }: { icon: typeof File; label: string; value: string; mono?: boolean; status?: string }) {
  return <div className="setup-row"><span className="setup-row-label"><Icon aria-hidden="true" />{label}</span><span className={mono ? "mono" : ""}>{value}</span><span className="setup-row-status"><span className="status-dot" aria-hidden="true" />{status}</span></div>;
}

function fileStatusLabel(status: "normal" | "missing" | "read-only") {
  return status === "normal" ? "正常" : status === "missing" ? "缺失" : "只读";
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
        <Route path="/" element={<SetupPage />} />
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
