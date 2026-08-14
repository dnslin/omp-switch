import { CheckCircle2, ChevronDown, CircleAlert, CircleCheck, File, Folder, Info, LayoutGrid, Server, Settings, SquareTerminal, Users } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useNavigate } from "react-router";
import { toast, Toaster } from "sonner";
import { RedetectionLoader } from "../components/redetection-loader";
import { Button, Card, NavigationItem, PageTitle, StatusIndicator } from "../components/ui";
import { asAppError, useTauriClient, type ConfigurationFileStatus, type OverviewDto, type OverviewModel, type OverviewProvider, type StartupState } from "../lib/tauri-client";


const pages = [
  { to: "/overview", label: "概览", icon: LayoutGrid },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/roles", label: "角色", icon: Users },
  { to: "/settings", label: "设置", icon: Settings },
] as const;
const REDETECT_MINIMUM_DURATION_MS = 1200;



type ShellStatus = { title: string; path: string; tone: "success" | "warning" | "danger" };

function MainShell({ children, status }: { children: React.ReactNode; status?: ShellStatus }) {
  const footer = status ?? { title: "尚未检测 OMP", path: "配置目录不可用", tone: "warning" as const };
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
          <Link className={`sidebar-footer sidebar-footer--${footer.tone}`} to="/settings" aria-label={`${footer.title}，${footer.path}`}>
            <strong><span className="status-dot" aria-hidden="true" />{footer.title}</strong>
            <code>{footer.path}</code>
          </Link>
        </aside>
        <section className="page-content">{children}</section>
      </main>
    </div>
  );
}

type ReadyState = Extract<StartupState, { kind: "omp-ready" }>;
type FailureState = Extract<StartupState, { kind: "invalid-executable" | "version-failed" | "config-path-failed" }>;
type RowStatus = { label: string; tone: "success" | "warning" | "danger" };

type TargetPresentation = {
  title: string;
  description: string;
  statusText: string;
  tone: "success" | "warning" | "danger";
  rowStatus: RowStatus;
  canEnter: boolean;
  needsExternalRepair: boolean;
  permissionSummary: string;
  retryLabel: "重新检测" | "重新读取";
  createLabel: string | null;
  enterLabel: string | null;
  extended: boolean;
  issueHeading: string | null;
};

type SetupPresentation = {
  readyState: ReadyState | null;
  failureState: FailureState | null;
  confirmingSwitch: boolean;
  targetPresentation: TargetPresentation | null;
  title: string;
  description: string;
  statusText: string;
  tone: "success" | "warning" | "danger";
};

function getTargetPresentation(target: ReadyState["targetConfiguration"], confirmingSwitch: boolean): TargetPresentation {
  switch (target.status) {
    case "writable":
      return confirmingSwitch
        ? { title: "确认切换 OMP", description: "请确认新的 OMP 及其 Target configuration；确认后才会替换当前选择。", statusText: "等待确认  ·  尚未切换 OMP", tone: "warning", rowStatus: { label: "正常", tone: "success" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置文件可读写，权限正常。", retryLabel: "重新检测", createLabel: null, enterLabel: "确认切换并进入应用", extended: false, issueHeading: null }
        : { title: "OMP 已找到", description: "OMP Switch 已确认可执行文件和权威配置目录。", statusText: "检测完成  ·  OMP 已可用", tone: "success", rowStatus: { label: "正常", tone: "success" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置文件可读写，权限正常。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入应用", extended: false, issueHeading: null };
    case "creation-required":
      return { title: "需要创建 OMP 配置", description: "请确认以下目录和最小配置文件。已有文件不会被覆盖。", statusText: "等待确认  ·  尚未创建配置", tone: "warning", rowStatus: { label: "待创建", tone: "warning" }, canEnter: false, needsExternalRepair: false, permissionSummary: "确认后将通过可恢复事务创建最小配置；中断将在下次检测时恢复。", retryLabel: "重新检测", createLabel: confirmingSwitch ? "确认切换并创建" : "创建", enterLabel: null, extended: true, issueHeading: null };
    case "read-only": {
      const yamlOnly = target.models.status === "alternate-only" || target.config.status === "alternate-only";
      return yamlOnly
        ? { title: "当前配置使用 .yaml", description: "OMP Switch MVP 只写入 .yml。当前配置可以查看，但不能修改。", statusText: "配置只读  ·  .yaml 不会被修改", tone: "warning", rowStatus: { label: "只读", tone: "warning" }, canEnter: true, needsExternalRepair: true, permissionSummary: "配置可查看；OMP Switch 不会修改当前文件。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入只读模式", extended: true, issueHeading: null }
        : { title: "Target configuration 只读", description: "当前目录或规范 .yml 文件不可写。配置可以查看，但不能修改。", statusText: "配置只读  ·  写入已禁用", tone: "warning", rowStatus: { label: "只读", tone: "warning" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置可查看；OMP Switch 不会修改当前文件。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入只读模式", extended: true, issueHeading: null };
    }
    case "migration-required":
      return { title: "需要先由 OMP 迁移配置", description: "请先使用当前 OMP 完成官方 YAML 迁移，然后重新检测。", statusText: "检测到旧 JSON  ·  不会创建空 YAML", tone: "warning", rowStatus: { label: "待迁移", tone: "warning" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新检测", createLabel: null, enterLabel: null, extended: true, issueHeading: null };
    case "parse-error": {
      const file = target.issue?.filePath.split(/[\\/]/).at(-1) ?? "YAML";
      return { title: `无法读取 ${file}`, description: "请在外部修复 YAML 后重新读取。OMP Switch 不会覆盖错误文件。", statusText: "YAML 解析失败  ·  写入已禁用", tone: "danger", rowStatus: { label: "格式错误", tone: "danger" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新读取", createLabel: null, enterLabel: null, extended: true, issueHeading: formatIssueLocation(target.issue?.line ?? null, target.issue?.column ?? null) };
    }
    case "unsafe":
      return { title: "无法安全访问 Target configuration", description: "链接、重解析点、路径类型或权限边界无法被安全确认。", statusText: "目标不安全  ·  写入已拒绝", tone: "danger", rowStatus: { label: "不安全", tone: "danger" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新检测", createLabel: null, enterLabel: null, extended: true, issueHeading: "无法确认真实目标。" };
  }
}

function getSetupPresentation(state: StartupState): SetupPresentation {
  const base = { confirmingSwitch: false, targetPresentation: null, tone: "warning" as const };
  switch (state.kind) {
    case "detecting":
      return { ...base, readyState: null, failureState: null, title: "正在检测 OMP…", description: "正在检查可执行文件、版本和权威配置目录。", statusText: "正在检测 OMP…" };
    case "omp-unavailable":
      return { ...base, readyState: null, failureState: null, title: "设置 OMP", description: state.message, statusText: state.message };
    case "invalid-executable":
    case "version-failed":
      return { ...base, readyState: null, failureState: state, title: "设置 OMP", description: state.message, statusText: state.message, tone: "danger" };
    case "config-path-failed":
      return { ...base, readyState: null, failureState: state, title: "无法获取 OMP 配置目录", description: state.message, statusText: state.message, tone: "danger" };
    case "omp-ready": {
      const confirmingSwitch = state.requiresConfirmation;
      const targetPresentation = getTargetPresentation(state.targetConfiguration, confirmingSwitch);
      return { readyState: state, failureState: null, confirmingSwitch, targetPresentation, ...targetPresentation };
    }
  }
}

function SetupPage() {
  const client = useTauriClient();
  const navigate = useNavigate();
  const [state, setState] = useState<StartupState>({ kind: "detecting" });
  const [redetecting, setRedetecting] = useState(false);
  const [initializing, setInitializing] = useState(false);
  const detectionInFlight = useRef(false);
  const initializationInFlight = useRef(false);
  useEffect(() => {
    let active = true;
    void client.getStartupState().then((next) => {
      if (active) setState(next);
    }).catch((error: unknown) => {
      const appError = asAppError(error, "无法读取启动状态");
      toast.error(appError.message, { description: appError.action });
      if (active) setState({ kind: "omp-unavailable", message: appError.message });
    });
    return () => { active = false; };
  }, [client]);

  async function detect() {
    if (detectionInFlight.current) return;
    detectionInFlight.current = true;
    const preserveReadyState = state.kind === "omp-ready";
    if (preserveReadyState) setRedetecting(true);
    else setState({ kind: "detecting" });
    try {
      let nextState: StartupState;
      if (preserveReadyState) {
        const redetection = state.requiresConfirmation
          ? client.validateSelectedOmp(state.executablePath)
          : client.detectOmp();
        const [detectionResult] = await Promise.allSettled([
          redetection,
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

  async function initializeConfiguration() {
    if (state.kind !== "omp-ready" || initializationInFlight.current) return;
    initializationInFlight.current = true;
    setInitializing(true);
    try {
      setState(await client.initializeTargetConfiguration(state.executablePath, {
        createPaths: state.targetConfiguration.createPaths,
        discoveryToken: state.targetConfiguration.discoveryToken,
      }));
    } catch (error: unknown) {
      const appError = asAppError(error, "无法创建最小 Target configuration");
      toast.error(appError.message, { description: appError.action });
      try {
        setState(await client.validateSelectedOmp(state.executablePath));
      } catch {
        setState({ kind: "omp-unavailable", message: appError.message });
      }
    }
    finally {
      initializationInFlight.current = false;
      setInitializing(false);
    }
  }

  async function openTargetDirectory() {
    if (state.kind !== "omp-ready") return;
    try {
      await client.openTargetConfigurationDirectory(state.executablePath);
    } catch (error: unknown) {
      const appError = asAppError(error, "无法打开配置目录");
      toast.error(appError.message, { description: appError.action });
    }
  }

  async function enterApplication() {
    if (state.kind !== "omp-ready") return;
    if (!getTargetPresentation(state.targetConfiguration, state.requiresConfirmation).canEnter) return;
    try {
      if (state.requiresConfirmation) await client.confirmSelectedOmp(state.executablePath);
      navigate("/overview");
    } catch (error: unknown) {
      const appError = asAppError(error, "无法保存 OMP 选择");
      toast.error(appError.message, { description: appError.action });
    }
  }

  const presentation = getSetupPresentation(state);
  const { readyState, failureState, confirmingSwitch, targetPresentation, title, description, statusText, tone } = presentation;
  const target = readyState?.targetConfiguration ?? null;

  return (
    <div className="app-frame">
      <main className={`setup-body ${targetPresentation?.extended ? "setup-body--extended" : ""} ${targetPresentation?.createLabel ? "setup-body--creation" : ""}`}>
        <section className="setup-card" aria-busy={redetecting || initializing}>
          <header><h1>{title}</h1><p>{description}</p></header>
          <div className={`setup-state setup-state--${tone}`} aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {statusText}
          </div>
          {readyState && target ? (
            <>
              <div className="setup-table">
                <SetupRow icon={SquareTerminal} label="可执行文件" value={readyState.executablePath} mono />
                <SetupRow icon={Info} label="版本" value={readyState.version} />
                <SetupRow icon={Folder} label="权威配置目录" value={displayTargetPath(target.path, target.resolvedPath)} mono status={targetPresentation?.rowStatus} />
                <SetupRow icon={File} label="models.yml" value={displayResolvedPath(target.models.canonicalPath, target.models.resolvedPath)} mono status={fileStatusView(target.models.status)} />
                <SetupRow icon={File} label="config.yml" value={displayResolvedPath(target.config.canonicalPath, target.config.resolvedPath)} mono status={fileStatusView(target.config.status)} />
              </div>
              {targetPresentation?.createLabel ? (
                <div className="setup-notice"><strong>将创建</strong><ul>{target.createPaths.map((path) => <li key={path}><code>{path}</code></li>)}</ul></div>
              ) : null}
              {target.warnings.length > 0 ? (
                <div className="setup-notice setup-notice--warning"><strong>注意</strong><ul>{target.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></div>
              ) : null}
              {target.recoveryNotice ? (
                <div className="setup-notice" role="status"><strong>已恢复上次中断操作</strong><p>{target.recoveryNotice}</p></div>
              ) : null}
              {target.issue ? (
                <div className="technical-details" role="alert">
                  <strong>{targetPresentation?.issueHeading}</strong>
                  <p className="mono">{target.issue.filePath}</p>
                  <p>{target.issue.message}</p>
                </div>
              ) : null}
              {confirmingSwitch ? (
                <div className="target-change" aria-label="Target configuration 变更">
                  <div><span>当前 Target configuration</span><code>{readyState.previousTargetConfiguration ?? "当前 OMP 无法读取，配置目录未知"}</code></div>
                  <div><span>切换后 Target configuration</span><code>{target.path}</code></div>
                </div>
              ) : null}
              <div className="permission-summary"><CheckCircle2 aria-hidden="true" />{targetPresentation?.permissionSummary}</div>
            </>
          ) : failureState ? (
            <details className="technical-details"><summary>查看技术详情</summary><p>诊断代码：{failureState.diagnosticCode}</p>{failureState.kind !== "invalid-executable" ? <><p>退出码：{failureState.exitCode ?? "不可用"}</p><p>{failureState.stderr || "命令没有返回 stderr。"}</p></> : null}</details>
          ) : null}
          <div className="setup-actions">
            <Button size="setup" variant="secondary" onClick={selectExecutable} disabled={state.kind === "detecting" || redetecting || initializing}>手动选择 OMP</Button>
            {readyState ? <Button size="setup" variant="secondary" onClick={detect} disabled={redetecting || initializing} disabledAppearance="stable">{targetPresentation?.retryLabel}</Button> : <Button size="setup" onClick={detect} disabled={state.kind === "detecting"}>自动检测</Button>}
            {targetPresentation?.needsExternalRepair ? <Button size="setup" variant="secondary" onClick={openTargetDirectory}>打开配置目录</Button> : null}
            {targetPresentation?.createLabel ? <Button size="setup" onClick={initializeConfiguration} disabled={initializing}>{initializing ? "创建中…" : targetPresentation.createLabel}</Button> : null}
            {targetPresentation?.canEnter && targetPresentation.enterLabel ? <Button size="setup" onClick={enterApplication} disabled={redetecting || initializing} disabledAppearance={redetecting ? "stable" : "muted"}>{targetPresentation.enterLabel}</Button> : null}
          </div>
          {redetecting ? (
            <div className="redetect-overlay" role="status" aria-live="polite" aria-label="正在重新检测 OMP" data-testid="redetect-progress">
              <div className="redetect-overlay__content"><RedetectionLoader /><strong>正在重新检测 OMP</strong></div>
            </div>
          ) : null}
        </section>
      </main>
    </div>
  );
}


function SetupRow({ icon: Icon, label, value, mono = false, status = { label: "正常", tone: "success" } }: { icon: typeof File; label: string; value: string; mono?: boolean; status?: RowStatus }) {
  return <div className="setup-row"><span className="setup-row-label"><Icon aria-hidden="true" />{label}</span><span className={mono ? "mono" : ""}>{value}</span><span className={`setup-row-status setup-row-status--${status.tone}`}><span className="status-dot" aria-hidden="true" />{status.label}</span></div>;
}

function displayResolvedPath(canonicalPath: string, resolvedPath: string | null) {
  return resolvedPath && resolvedPath !== canonicalPath ? resolvedPath : "";
}
function displayTargetPath(path: string, resolvedPath: string | null) {
  return resolvedPath && resolvedPath !== path ? `${path} → ${resolvedPath}` : path;
}


const FILE_STATUS_VIEW: Record<ConfigurationFileStatus, RowStatus> = {
  normal: { label: "正常", tone: "success" },
  missing: { label: "缺失", tone: "warning" },
  "read-only": { label: "只读", tone: "warning" },
  "alternate-only": { label: ".yaml 只读", tone: "warning" },
  "canonical-with-alternate": { label: "正常 · 有 .yaml", tone: "success" },
  "legacy-json": { label: "旧 JSON", tone: "warning" },
  "parse-error": { label: "格式错误", tone: "danger" },
  unsafe: { label: "不安全", tone: "danger" },
};

function fileStatusView(status: ConfigurationFileStatus) {
  return FILE_STATUS_VIEW[status];
}

function formatIssueLocation(line: number | null, column: number | null) {
  if (line !== null && column !== null) return `第 ${line} 行，第 ${column} 列附近存在格式错误。`;
  if (line !== null) return `第 ${line} 行附近存在格式错误。`;
  return "YAML 存在格式错误。";
}
function overviewShellStatus(data: OverviewDto): ShellStatus {
  return {
    title: `OMP 已连接  ·  ${formatOverviewVersion(data.omp.version)}`,
    path: data.targetConfiguration.resolvedPath ?? data.targetConfiguration.path,
    tone: data.state === "read-only" ? "warning" : "success",
  };
}

function OverviewPage() {
  const client = useTauriClient();
  const [data, setData] = useState<OverviewDto | null>(null);
  const [error, setError] = useState<ReturnType<typeof asAppError> | null>(null);
  const [loading, setLoading] = useState(true);
  const requestId = useRef(0);

  async function reload() {
    const currentRequest = ++requestId.current;
    setLoading(true);
    setData(null);
    setError(null);
    try {
      const next = await client.getOverview();
      if (currentRequest === requestId.current) setData(next);
    } catch (cause: unknown) {
      if (currentRequest === requestId.current) setError(asAppError(cause, "无法读取概览"));
    } finally {
      if (currentRequest === requestId.current) setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
    return () => { requestId.current += 1; };
  }, [client]);

  const pageClass = loading ? "overview-page--loading" : error || !data ? "overview-page--error" : `overview-page--${data.state}`;
  return (
    <MainShell status={data ? overviewShellStatus(data) : undefined}>
      <div className={`overview-page ${pageClass}`} aria-busy={loading}>
        <OverviewPageHeader />
        {loading ? <OverviewLoadingBody /> : error ? <OverviewErrorBody error={error} onReload={reload} /> : data ? <OverviewContentBody data={data} /> : <OverviewErrorBody error={{ message: "OMP 没有返回概览数据。", action: "请重新读取；如果问题持续，请查看脱敏日志。" }} onReload={reload} />}
      </div>
    </MainShell>
  );
}

function OverviewPageHeader() {
  return (
    <header className="page-title overview-page-header">
      <h1>概览</h1>
      <p>查看当前配置状态并快速验证模型连接。</p>
    </header>
  );
}

function OverviewLoadingBody() {
  return (
    <div className="overview-loading" role="status" aria-label="正在读取配置" aria-live="polite">
      <strong>正在读取配置…</strong>
      <div className="overview-skeleton overview-skeleton--sync" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--environment" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--metrics" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--test" aria-hidden="true" />
    </div>
  );
}

function OverviewErrorBody({ error, onReload }: { error: { message: string; action: string }; onReload: () => Promise<void> }) {
  return (
    <section className="overview-error-card" role="alert" aria-live="assertive">
      <CircleAlert aria-hidden="true" />
      <div>
        <h2>无法读取概览</h2>
        <p>{error.message}</p>
        <p className="overview-state-detail">{error.action}</p>
      </div>
      <Button variant="secondary" onClick={() => void onReload()}>重新读取</Button>
    </section>
  );
}

function OverviewContentBody({ data }: { data: OverviewDto }) {
  const selectedModel = data.models.find((model) => model.complete) ?? data.models[0] ?? null;
  const selectedProvider = selectedModel ? data.providers.find((provider) => provider.id === selectedModel.providerId) ?? null : data.providers[0] ?? null;
  return (
    <>
      <OverviewSyncStrip data={data} />
      <OverviewEnvironment data={data} />
      <OverviewMetrics data={data} />
      {data.state === "empty" || data.state === "read-only" ? <OverviewStateBanner data={data} /> : null}
      <div className="overview-test-area">
        <QuickTestPanel data={data} provider={selectedProvider} model={selectedModel} />
        <TestResultPanel />
      </div>
    </>
  );
}

function OverviewSyncStrip({ data }: { data: OverviewDto }) {
  return (
    <section className="overview-sync-strip" aria-label="配置同步状态">
      <OverviewFileStatus name="models.yml" file={data.files.models} />
      <OverviewFileStatus name="config.yml" file={data.files.config} />
      <div><span>最近备份  —</span></div>
    </section>
  );
}

function OverviewFileStatus({ name, file }: { name: string; file: OverviewDto["files"]["models"] }) {
  const status = fileStatusView(file.status);
  const synced = file.contentHash !== null && (file.status === "normal" || file.status === "canonical-with-alternate");
  const tone = synced ? "success" : status.tone;
  const StatusIcon = synced ? CircleCheck : tone === "danger" ? CircleAlert : Info;
  return (
    <div className={`overview-sync-file overview-sync-file--${tone}`}>
      <span>{name}  {synced ? "已同步" : status.label}</span>
      <StatusIcon aria-hidden="true" className={`overview-file-status-icon overview-file-status-icon--${tone}`} />
    </div>
  );
}

function OverviewEnvironment({ data }: { data: OverviewDto }) {
  const targetPath = data.targetConfiguration.resolvedPath ?? data.targetConfiguration.path;
  return (
    <section className="overview-environment" aria-label="OMP 环境">
      <div className="overview-environment-cell"><span>OMP 环境</span><span className="overview-environment-value"><span className="status-dot" aria-hidden="true" />已连接</span></div>
      <div className="overview-environment-cell"><span>版本</span><span className="overview-environment-value">{data.omp.version}</span></div>
      <div className="overview-environment-cell"><span>可执行文件</span><code>{data.omp.executablePath}</code></div>
      <div className="overview-environment-cell"><span>权威配置目录</span><code>{targetPath}</code></div>
    </section>
  );
}

function OverviewMetrics({ data }: { data: OverviewDto }) {
  return (
    <section className="overview-metrics" aria-label="配置统计">
      <div><span>自定义 Provider</span><strong>{formatOverviewCount(data.counts.providerCount)}</strong></div>
      <div><span>模型</span><strong>{formatOverviewCount(data.counts.modelCount)}</strong></div>
      <div><span>已配置角色</span><strong>{formatOverviewCount(data.counts.roleCount)}</strong></div>
    </section>
  );
}

function OverviewStateBanner({ data }: { data: OverviewDto }) {
  const empty = data.state === "empty";
  return (
    <section className={`overview-state-banner overview-state-banner--${empty ? "empty" : "readonly"}`} aria-live="polite">
      {empty ? <CircleCheck aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}
      <div>
        <strong>{empty ? "还没有可管理的自定义 Provider" : "配置只读"}</strong>
        <p>{empty ? (data.emptyReason ?? "创建一个 Provider，并同时配置它的第一个模型。") : (data.readOnlyReason ?? "当前配置只能查看；OMP Switch 不会修改配置文件。")}</p>
        {data.nextAction ? <p className="overview-state-detail">{data.nextAction}</p> : null}
      </div>
    </section>
  );
}

function QuickTestPanel({ data, provider, model }: { data: OverviewDto; provider: OverviewProvider | null; model: OverviewModel | null }) {
  const canTest = data.state === "normal" && model?.complete === true && model.editable;
  const finalAddress = provider && model ? modelEndpoint(provider, model) : "—";
  const protocol = model?.effectiveApi ? `${model.effectiveApi}  ·  ${model.apiSource === "provider" ? "Provider 默认值" : "模型指定"}` : "—";
  const capabilities = model?.input.map((input) => input === "text" ? "Text" : input === "image" ? "Image" : input).join("  ·  ") || "—";
  return (
    <section className="overview-panel overview-quick-test" aria-label="快速测试">
      <h2>快速测试</h2>
      <OverviewField label="Provider" value={provider?.id ?? "暂无 Provider"} select />
      <OverviewField label="模型" value={model?.id ?? "暂无模型"} select />
      <OverviewField label="有效协议" value={protocol} />
      <OverviewField label="最终地址" value={finalAddress} mono />
      <OverviewField label="能力" value={capabilities} />
      <OverviewField label="Context Window" value={model?.contextWindow ? formatOverviewCount(model.contextWindow) : "—"} />
      <div className="overview-panel-actions"><Button disabled={!canTest} aria-label="测试模型">测试模型</Button></div>
    </section>
  );
}

function OverviewField({ label, value, select = false, mono = false }: { label: string; value: string; select?: boolean; mono?: boolean }) {
  return (
    <div className="overview-field">
      <span>{label}</span>
      <div className={`overview-field-control ${mono ? "overview-field-control--mono" : ""}`}>
        <span>{value}</span>
        {select ? <ChevronDown aria-hidden="true" /> : null}
      </div>
    </div>
  );
}

function TestResultPanel() {
  return (
    <section className="overview-panel overview-result" aria-label="测试结果">
      <header><h2>测试结果</h2><span className="overview-result-status"><span className="status-dot" aria-hidden="true" />尚未测试</span></header>
      <OverviewResultRow label="模型" value="—" />
      <OverviewResultRow label="耗时" value="—" />
      <OverviewResultRow label="状态码" value="—" />
      <OverviewResultRow label="时间" value="—" />
    </section>
  );
}

function OverviewResultRow({ label, value }: { label: string; value: string }) {
  return <div className="overview-result-row"><span>{label}</span><span>{value}</span></div>;
}

function OverviewSkeleton() {
  return <div className="overview-skeleton" aria-hidden="true" />;
}

function formatOverviewCount(value: number) {
  return new Intl.NumberFormat("zh-CN").format(value);
}

function modelEndpoint(provider: OverviewProvider, model: OverviewModel) {
  const base = provider.baseUrl?.replace(/\/+$/, "") ?? "";
  if (!base || !model.effectiveApi) return "—";
  switch (model.effectiveApi) {
    case "openai-completions": return `${base}/chat/completions`;
    case "openai-responses": return `${base}/responses`;
    case "anthropic-messages": return `${base}/v1/messages`;
    case "google-generative-ai": return `${base}/models/${encodeURIComponent(model.id)}:streamGenerateContent?alt=sse`;
    default: return "—";
  }
}
function formatOverviewVersion(version: string) {
  const normalized = version.trim();
  return /^(?:v|omp\/)/i.test(normalized) ? normalized : `v${normalized}`;
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
        <Route path="/overview" element={<OverviewPage />} />
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
