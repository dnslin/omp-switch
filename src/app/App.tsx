import { CheckCircle2, File, Folder, Info, SquareTerminal } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, Navigate, Route, Routes, useNavigate, useParams } from "react-router";
import { toast, Toaster } from "sonner";
import { RedetectionLoader } from "../components/redetection-loader";
import { Button, Card, PageTitle, StatusIndicator } from "../components/ui";
import { asAppError, useTauriClient, type StartupState } from "../lib/tauri-client";
import { useUiSettings } from "../store/ui-settings";
import { MainShell } from "./MainShell";
import { OverviewPage } from "./OverviewPage";
import { ProvidersPage } from "./ProvidersPage";
import { useOverviewLoad } from "./overview-load";
import { fileStatusView, providerAuthSummary, startupShellStatus, targetConfigurationStatusView, type RowStatus } from "./omp-presentation";


const REDETECT_MINIMUM_DURATION_MS = 1200;




type ReadyState = Extract<StartupState, { kind: "omp-ready" }>;
type FailureState = Extract<StartupState, { kind: "invalid-executable" | "version-failed" | "config-path-failed" }>;

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
  const targetStatus = targetConfigurationStatusView(target.status);
  switch (target.status) {
    case "writable":
      return confirmingSwitch
        ? { title: "确认切换 OMP", description: "请确认新的 OMP 及其 Target configuration；确认后才会替换当前选择。", statusText: "等待确认  ·  尚未切换 OMP", tone: "warning", rowStatus: { label: "正常", tone: "success" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置文件可读写，权限正常。", retryLabel: "重新检测", createLabel: null, enterLabel: "确认切换并进入应用", extended: false, issueHeading: null }
        : { title: "OMP 已找到", description: "OMP Switch 已确认可执行文件和权威配置目录。", statusText: "检测完成  ·  OMP 已可用", tone: targetStatus.tone, rowStatus: { label: "正常", tone: "success" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置文件可读写，权限正常。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入应用", extended: false, issueHeading: null };
    case "creation-required":
      return { title: "需要创建 OMP 配置", description: "请确认以下目录和最小配置文件。已有文件不会被覆盖。", statusText: "等待确认  ·  尚未创建配置", tone: targetStatus.tone, rowStatus: { label: "待创建", tone: "warning" }, canEnter: false, needsExternalRepair: false, permissionSummary: "确认后将通过可恢复事务创建最小配置；中断将在下次检测时恢复。", retryLabel: "重新检测", createLabel: confirmingSwitch ? "确认切换并创建" : "创建", enterLabel: null, extended: true, issueHeading: null };
    case "read-only": {
      const yamlOnly = target.models.status === "alternate-only" || target.config.status === "alternate-only";
      return yamlOnly
        ? { title: "当前配置使用 .yaml", description: "OMP Switch MVP 只写入 .yml。当前配置可以查看，但不能修改。", statusText: "配置只读  ·  .yaml 不会被修改", tone: targetStatus.tone, rowStatus: { label: "只读", tone: "warning" }, canEnter: true, needsExternalRepair: true, permissionSummary: "配置可查看；OMP Switch 不会修改当前文件。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入只读模式", extended: true, issueHeading: null }
        : { title: "Target configuration 只读", description: "当前目录或规范 .yml 文件不可写。配置可以查看，但不能修改。", statusText: "配置只读  ·  写入已禁用", tone: targetStatus.tone, rowStatus: { label: "只读", tone: "warning" }, canEnter: true, needsExternalRepair: false, permissionSummary: "配置可查看；OMP Switch 不会修改当前文件。", retryLabel: "重新检测", createLabel: null, enterLabel: "进入只读模式", extended: true, issueHeading: null };
    }
    case "migration-required":
      return { title: "需要先由 OMP 迁移配置", description: "请先使用当前 OMP 完成官方 YAML 迁移，然后重新检测。", statusText: "检测到旧 JSON  ·  不会创建空 YAML", tone: targetStatus.tone, rowStatus: { label: "待迁移", tone: "warning" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新检测", createLabel: null, enterLabel: null, extended: true, issueHeading: null };
    case "parse-error": {
      const file = target.issue?.filePath.split(/[\\/]/).at(-1) ?? "YAML";
      return { title: `无法读取 ${file}`, description: "请在外部修复 YAML 后重新读取。OMP Switch 不会覆盖错误文件。", statusText: "YAML 解析失败  ·  写入已禁用", tone: targetStatus.tone, rowStatus: { label: "格式错误", tone: "danger" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新读取", createLabel: null, enterLabel: null, extended: true, issueHeading: formatIssueLocation(target.issue?.line ?? null, target.issue?.column ?? null) };
    }
    case "unsafe":
      return { title: "无法安全访问 Target configuration", description: "链接、重解析点、路径类型或权限边界无法被安全确认。", statusText: "目标不安全  ·  写入已拒绝", tone: targetStatus.tone, rowStatus: { label: "不安全", tone: "danger" }, canEnter: false, needsExternalRepair: true, permissionSummary: "配置目录已确认；写入保持禁用，直到外部问题解决。", retryLabel: "重新检测", createLabel: null, enterLabel: null, extended: true, issueHeading: "无法确认真实目标。" };
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



function formatIssueLocation(line: number | null, column: number | null) {
  if (line !== null && column !== null) return `第 ${line} 行，第 ${column} 列附近存在格式错误。`;
  if (line !== null) return `第 ${line} 行附近存在格式错误。`;
  return "YAML 存在格式错误。";
}




const routeCopy = {
  roles: ["角色", "管理 OMP 模型角色。", "角色管理将在后续工单中实现。"],
  settings: ["设置", "配置 OMP 路径、主题与轻量界面偏好。", "设置能力将在后续工单中扩展；当前不会保存任何 Provider、Model definition、Model role 或 Direct API Key。"],
} as const;

function SettingsPage() {
  const client = useTauriClient();
  const [state, setState] = useState<StartupState>({ kind: "detecting" });

  useEffect(() => {
    let active = true;
    void client.getStartupState().then((next) => {
      if (active) setState(next);
    }).catch((cause: unknown) => {
      if (active) setState({ kind: "omp-unavailable", message: asAppError(cause, "无法读取 OMP 状态").message });
    });
    return () => { active = false; };
  }, [client]);

  const ready = state.kind === "omp-ready";
  const targetStatus = ready ? targetConfigurationStatusView(state.targetConfiguration.status) : null;
  const statusText = targetStatus?.label
    ?? (state.kind === "detecting"
      ? "正在检测 OMP…"
      : "message" in state
        ? state.message
        : "OMP 状态不可用");
  return (
    <MainShell status={startupShellStatus(state)}>
      <PageTitle title="设置" description="配置 OMP 路径、主题与轻量界面偏好。" />
      <section id="omp-settings" className="settings-section" aria-labelledby="omp-settings-title">
        <h2 id="omp-settings-title">OMP 与 Target configuration</h2>
        <StatusIndicator tone={targetStatus?.tone ?? "warning"}>{statusText}</StatusIndicator>
        {ready ? (
          <div className="settings-details">
            <p><strong>版本</strong><span>{state.version}</span></p>
            <p><strong>可执行文件</strong><code>{state.executablePath}</code></p>
            <p><strong>权威配置目录</strong><code>{state.targetConfiguration.resolvedPath ?? state.targetConfiguration.path}</code></p>
          </div>
        ) : <p className="placeholder-card">完成 OMP 检测后，这里会显示权威配置目录和文件状态。</p>}
      </section>
    </MainShell>
  );
}

const providerDetailLoadCopy = {
  missingOverview: {
    code: "provider-detail-missing-overview",
    message: "OMP 没有返回 Provider 详情所需的数据。",
    action: "请重新读取；如果问题持续，请查看脱敏日志。",
  },
  requestFailure: "无法读取 Provider 详情",
};

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
  const { providerId } = useParams();
  const { data, error, loading, reload, shellStatus } = useOverviewLoad(providerDetailLoadCopy);
  const provider = data?.providers.find((item) => item.id === providerId);
  const authSummary = provider ? providerAuthSummary(provider) : "不支持的认证";

  return (
    <MainShell status={shellStatus}>
      <main className="provider-detail-page" aria-busy={loading}>
        <Link className="provider-detail-back" to="/providers">← <span>Providers</span></Link>
        {loading ? (
          <section className="provider-detail-loading" role="status" aria-live="polite">正在读取 Provider…</section>
        ) : error ? (
          <section className="provider-detail-error" role="alert" aria-live="assertive">
            <div><h1>无法读取 Provider</h1><p>{error.message}</p><p>{error.action}</p></div>
            <Button type="button" variant="secondary" onClick={() => void reload()}>重新读取</Button>
          </section>
        ) : !provider ? (
          <section className="provider-detail-error" role="alert">
            <div><h1>Provider 不存在</h1><p>该 Provider 可能已被外部删除或更名。</p></div>
            <Button asChild variant="secondary"><Link to="/providers">返回 Providers</Link></Button>
          </section>
        ) : (
          <>
            <header className="provider-detail-heading">
              <h1>{provider.id}</h1>
              <code>{provider.baseUrl ?? "未配置地址"}</code>
            </header>
            <section className="provider-detail-summary" aria-label="Provider 摘要">
              <span>默认协议</span><strong>{provider.defaultApi ?? "由模型指定"}</strong>
              <span>认证</span><strong>{authSummary}</strong>
              <span>模型</span><strong>{provider.modelCount}</strong>
              <span>状态</span><StatusIndicator tone={provider.editable ? "success" : "warning"}>{provider.editable ? "正常" : "只读"}</StatusIndicator>
            </section>
            <section className="provider-detail-models" aria-labelledby="provider-detail-models-title">
              <h2 id="provider-detail-models-title">模型</h2>
              <table>
                <thead><tr><th scope="col">名称 / Model ID</th><th scope="col">有效协议</th><th scope="col">能力</th><th scope="col">Context</th><th scope="col">Max Tokens</th></tr></thead>
                <tbody>
                  {provider.models.map((model) => (
                    <tr key={model.id}>
                      <td><strong>{model.name ?? "未命名模型"}</strong><code>{model.id}</code></td>
                      <td>{model.effectiveApi ?? "未配置"}</td>
                      <td>{model.input.join(", ") || "未配置"}</td>
                      <td>{model.contextWindow?.toLocaleString() ?? "未配置"}</td>
                      <td>{model.maxTokens?.toLocaleString() ?? "未配置"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          </>
        )}
      </main>
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
  const beginHydration = useUiSettings((state) => state.beginHydration);
  const hydrate = useUiSettings((state) => state.hydrate);
  const failHydration = useUiSettings((state) => state.failHydration);

  useEffect(() => {
    let active = true;
    beginHydration();
    void client.getUiSettings().then((settings) => {
      if (active) hydrate(settings);
    }).catch((error: unknown) => {
      if (!active) return;
      failHydration();
      const appError = asAppError(error, "无法读取界面状态");
      toast.error(appError.message, { description: appError.action });
    });
    return () => { active = false; };
  }, [beginHydration, client, failHydration, hydrate]);

  return (
    <div className="window">
      <Routes>
        <Route path="/" element={<SetupPage />} />
        <Route path="/setup" element={<SetupPage />} />
        <Route path="/overview" element={<OverviewPage />} />
        <Route path="/providers" element={<ProvidersPage />} />
        <Route path="/providers/:providerId" element={<ProviderDetailPage />} />
        <Route path="/roles" element={<PlaceholderPage page="roles" />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Routes>
      <Toaster position="bottom-right" richColors />
    </div>
  );
}
