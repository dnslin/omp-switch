import { CheckCircle2, Copy, File, Folder, Info, MoreHorizontal, Pencil, SquareTerminal, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, Navigate, Route, Routes, useNavigate, useParams } from "react-router";
import { toast, Toaster } from "sonner";
import { RedetectionLoader } from "../components/redetection-loader";
import { Button, Card, ConfirmDialog, PageTitle, SearchInput, StatusIndicator } from "../components/ui";
import { asAppError, useTauriClient, type AppError, type OverviewModel, type StartupState } from "../lib/tauri-client";
import { useModelTestStore } from "../store/model-test";
import { useUiSettings } from "../store/ui-settings";
import { MainShell } from "./MainShell";
import { ModelCreateSheet } from "./ModelCreateSheet";
import { OverviewPage } from "./OverviewPage";
import { ProviderEditDialog } from "./ProviderEditDialog";
import { ProvidersPage } from "./ProvidersPage";
import { ModelRolesPage } from "./ModelRolesPage";
import { useOverviewLoad } from "./overview-load";
import { buildModelEndpoint } from "./model-endpoint";
import { isModelTestable, useModelTestRunner, useRefreshAfterModelTest } from "./model-test";
import { fileStatusView, providerAuthSummary, startupShellStatus, targetConfigurationStatusView, type RowStatus } from "./omp-presentation";


const REDETECT_MINIMUM_DURATION_MS = 1200;
const MODEL_TEST_STATE_POLL_MS = 250;




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
                <div className="setup-notice" role={target.status === "unsafe" ? "alert" : "status"}>
                  <strong>{target.status === "unsafe" ? "上次事务需要人工处理" : "已恢复上次中断操作"}</strong>
                  <p>{target.recoveryNotice}</p>
                </div>
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
function uniqueReferences(paths: string[]): string[] {
  return [...new Set(paths)];
}
function isHashConflict(error: AppError | null): boolean {
  return error?.code === "models-hash-conflict" || error?.code === "config-hash-conflict";
}

function DeletionImpact({
  objectLabel,
  includedModels,
  roleReferences,
  otherReferences,
  blockedReason,
  crossFileTransaction,
  onOpenTargetDirectory,
  conflictError,
  onReload,
}: {
  objectLabel: string;
  includedModels: string[];
  roleReferences: string[];
  otherReferences: string[];
  blockedReason: string | null;
  crossFileTransaction: boolean;
  onOpenTargetDirectory?: () => Promise<void>;
  conflictError: AppError | null;
  onReload?: () => Promise<void>;
}) {
  const renderPaths = (paths: string[]) => paths.length ? (
    <ul className="deletion-impact__list">
      {paths.map((path) => <li key={path}><code>{path}</code></li>)}
    </ul>
  ) : <p className="deletion-impact__empty">无</p>;
  return (
    <div className="deletion-impact">
      <dl className="deletion-impact__summary">
        <div><dt>删除对象</dt><dd><code>{objectLabel}</code></dd></div>
        <div><dt>包含模型</dt><dd>{includedModels.length ? includedModels.map((model) => <code key={model}>{model}</code>) : <span className="deletion-impact__empty">无</span>}</dd></div>
      </dl>
      <section className="deletion-impact__section">
        <h3>{crossFileTransaction ? "将清除的受支持 Model role" : "受影响 Model role"}</h3>
        {renderPaths(roleReferences)}
      </section>
      <section className="deletion-impact__section">
        <h3>其他引用</h3>
        {renderPaths(otherReferences)}
      </section>
      {conflictError ? (
        <section className="deletion-impact__blocked" role="alert">
          <strong>配置冲突</strong>
          <p>{conflictError.message}</p>
          <p>{conflictError.action}</p>
          {onReload ? <Button type="button" variant="secondary" onClick={() => void onReload()}>重新读取</Button> : null}
        </section>
      ) : blockedReason ? (
        <section className="deletion-impact__blocked" role="status">
          <strong>删除已阻止</strong>
          <p>{blockedReason}</p>
          <p className="deletion-impact__backup">当前不会写入配置，也不会创建备份。</p>
          {onOpenTargetDirectory ? <Button type="button" variant="secondary" onClick={() => void onOpenTargetDirectory()}>打开配置目录</Button> : null}
        </section>
      ) : (
        <p className="deletion-impact__backup">
          {crossFileTransaction
            ? <>将通过同一 Configuration transaction 备份并修改 <code>models.yml</code> 与 <code>config.yml</code>。</>
            : <>此操作会创建备份（<code>models.yml</code>）。</>}
        </p>
      )}
    </div>
  );
}
function ProviderDetailPage() {
  const { providerId } = useParams();
  const client = useTauriClient();
  const navigate = useNavigate();
  const modelTest = useModelTestRunner();
  const { data, startupState, error, loading, revision, reload, refresh, shellStatus } = useOverviewLoad(providerDetailLoadCopy);
  useRefreshAfterModelTest({ ready: Boolean(data), loading, revision, refresh });
  const [editing, setEditing] = useState(false);
  const [editingModelsHash, setEditingModelsHash] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [modelEditor, setModelEditor] = useState<{ mode: "create" | "edit" | "view"; model?: OverviewModel; copy?: boolean } | null>(null);
  const [modelEditorModelsHash, setModelEditorModelsHash] = useState<string | null>(null);
  const [deletingModel, setDeletingModel] = useState<OverviewModel | null>(null);
  const [deletingProvider, setDeletingProvider] = useState(false);
  const [deleteHashes, setDeleteHashes] = useState<{ models: string; config: string } | null>(null);
  const [deleteError, setDeleteError] = useState<ReturnType<typeof asAppError> | null>(null);
  const [providerDeleteError, setProviderDeleteError] = useState<ReturnType<typeof asAppError> | null>(null);
  const [openModelActions, setOpenModelActions] = useState<string | null>(null);
  const provider = data?.providers.find((item) => item.id === providerId);
  const authSummary = provider ? providerAuthSummary(provider) : "不支持的认证";
  const latestResult = provider && modelTest.result?.providerId === provider.id ? modelTest.result : null;
  const latestTerminal = provider && modelTest.terminal?.providerId === provider.id ? modelTest.terminal : null;
  const latestModel = latestResult
    ? provider?.models.find((model) => model.id === latestResult.modelId) ?? null
    : latestTerminal
      ? provider?.models.find((model) => model.id === latestTerminal.modelId) ?? null
      : null;
  const activeProvider = data?.providers.find((item) => item.id === modelTest.activeProviderId) ?? null;
  const activeModel = activeProvider && modelTest.activeModelId ? activeProvider.models.find((model) => model.id === modelTest.activeModelId) ?? null : null;
  const latestEndpoint = latestResult && latestModel && !latestModel.hasBaseUrlOverride
    ? buildModelEndpoint(provider?.baseUrl, latestModel.id, latestResult.protocol)
    : { kind: "not-configured" as const };
  const openedModelsHash = data?.files.models.contentHash ?? null;
  const openedConfigHash = data?.files.config.contentHash ?? null;
  const targetWritable = data?.targetConfiguration.writable ?? false;
  const canManageModels = Boolean(provider?.editable && openedModelsHash && openedConfigHash && targetWritable);
  const providerRoleReferences = uniqueReferences(provider?.roleReferencePaths ?? []);
  const providerOtherReferences = uniqueReferences(provider?.otherReferencePaths ?? []);
  const modelRoleReferences = uniqueReferences(deletingModel?.roleReferencePaths ?? []);
  const modelOtherReferences = uniqueReferences(deletingModel?.otherReferencePaths ?? []);
  const providerReadOnlyModelId = provider?.models.find((model) => !model.editable)?.id ?? null;

  const modelDeleteBlockedReason = deletingModel && provider
    ? modelOtherReferences.length > 0
      ? "OMP Switch 不会修改非受管配置路径；请先在 OMP 或外部编辑器中处理这些引用。"
      : provider.modelCount <= 1
        ? "这是 Provider 下的最后一个 Model definition；请转入 Provider 删除流程。"
        : null
    : null;
  const providerDeleteBlockedReason = providerOtherReferences.length > 0
    ? "OMP Switch 不会修改非受管配置路径；请先在 OMP 或外部编辑器中处理这些引用。"
    : providerReadOnlyModelId
      ? `Provider 包含只读 Model definition ${providerReadOnlyModelId}；请先处理该模型，OMP Switch 不会通过删除 Provider 绕过只读边界。`
      : provider?.modelCount === 0
        ? "Provider 没有可删除的 Model definition，当前配置不符合 Custom Provider 结构。"
        : null;
  const openTargetDirectory = useCallback(async () => {
    const executablePath = startupState?.kind === "omp-ready" ? startupState.executablePath : data?.omp.executablePath;
    if (!executablePath) {
      toast.error("无法打开配置目录", { description: "当前 OMP 路径不可用，请重新检测。" });
      return;
    }
    try {
      await client.openTargetConfigurationDirectory(executablePath);
    } catch (cause: unknown) {
      const appError = asAppError(cause, "无法打开配置目录");
      toast.error(appError.message, { description: appError.action });
    }
  }, [client, data?.omp.executablePath, startupState]);
  const canOpenModelTargetDirectory = modelRoleReferences.length > 0 || modelOtherReferences.length > 0;
  const canOpenProviderTargetDirectory = providerRoleReferences.length > 0 || providerOtherReferences.length > 0;

  const openModelEditor = (editor: NonNullable<typeof modelEditor>) => {
    if (!openedModelsHash) return;
    setModelEditorModelsHash(openedModelsHash);
    setModelEditor(editor);
  };
  const dismissModelEditor = () => {
    setModelEditor(null);
    setModelEditorModelsHash(null);
  };
  const dismissProviderEditor = () => {
    setEditing(false);
    setEditingModelsHash(null);
  };

  const normalizedSearch = modelSearch.trim().toLocaleLowerCase();
  const models = provider?.models.filter((model) => [
    model.id,
    model.name,
    model.effectiveApi,
    model.readOnlyReason,
    model.status,
    ...model.referencePaths,
  ].some((value) => value?.toLocaleLowerCase().includes(normalizedSearch))) ?? [];

  const saveModel = async () => {
    if (!modelEditor || !modelEditorModelsHash || !provider) return null;
    const editorMode = modelEditor.mode;
    const reloadError = await reload();
    if (reloadError) return reloadError;
    dismissModelEditor();
    toast.success(editorMode === "edit" ? "Model 已保存" : "Model 已创建");
    return null;
  };
  const reloadModelAfterConflict = async () => {
    const reloadError = await reload();
    if (reloadError) {
      toast.error(reloadError.message, { description: reloadError.action });
      return;
    }
    setDeletingModel(null);
    setDeleteHashes(null);
    setDeleteError(null);
  };

  const reloadProviderAfterConflict = async () => {
    const reloadError = await reload();
    if (reloadError) {
      toast.error(reloadError.message, { description: reloadError.action });
      return;
    }
    setDeletingProvider(false);
    setDeleteHashes(null);
    setProviderDeleteError(null);
  };

  const deleteModel = async () => {
    if (!deletingModel || !deleteHashes || !provider) return;
    try {
      await client.deleteModel({
        openedModelsHash: deleteHashes.models,
        openedConfigHash: deleteHashes.config,
        providerId: provider.id,
        modelId: deletingModel.id,
      });
      setDeletingModel(null);
      setDeleteHashes(null);
      setDeleteError(null);
      const reloadError = await reload();
      if (reloadError) {
        setDeleteError(reloadError);
        return;
      }
      toast.success("Model 已删除");
    } catch (cause: unknown) {
      const appError = asAppError(cause, "删除 Model 失败");
      if (isHashConflict(appError)) {
        setDeleteError(appError);
        return;
      }
      setDeletingModel(null);
      setDeleteHashes(null);
      setDeleteError(appError);
    }
  };
  const deleteProvider = async () => {
    if (!deletingProvider || !deleteHashes || !provider) return;
    try {
      await client.deleteProvider({
        openedModelsHash: deleteHashes.models,
        openedConfigHash: deleteHashes.config,
        providerId: provider.id,
      });
      setDeletingProvider(false);
      setDeleteHashes(null);
      setProviderDeleteError(null);
      const reloadError = await reload();
      if (reloadError) {
        setProviderDeleteError(reloadError);
        return;
      }
      toast.success("Provider 已删除");
      navigate("/providers");
    } catch (cause: unknown) {
      const appError = asAppError(cause, "删除 Provider 失败");
      if (isHashConflict(appError)) {
        setProviderDeleteError(appError);
        return;
      }
      setDeletingProvider(false);
      setDeleteHashes(null);
      setProviderDeleteError(appError);
    }
  };

  return (
    <MainShell status={shellStatus}>
      <main className="provider-detail-page" aria-busy={loading}>
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
            <header className="provider-detail-top">
              <div className="provider-detail-identity">
                <Link className="provider-detail-back" to="/providers">← <span>Providers</span></Link>
                <h1>{provider.id}</h1>
                <code>{provider.baseUrl ?? "未配置地址"}</code>
              </div>
              <div className="provider-detail-actions">
                <Button type="button" disabled={!provider.editable || !openedModelsHash || !targetWritable} onClick={() => { if (!openedModelsHash) return; setEditingModelsHash(openedModelsHash); setEditing(true); }}>编辑 Provider</Button>
                <Button type="button" variant="secondary" className="provider-detail-delete" disabled={!provider.editable || !openedModelsHash || !openedConfigHash || !targetWritable} onClick={() => { if (!openedModelsHash || !openedConfigHash) return; setProviderDeleteError(null); setDeleteHashes({ models: openedModelsHash, config: openedConfigHash }); setDeletingProvider(true); }}>删除 Provider</Button>
              </div>
            </header>
            {!provider.editable ? <p className="provider-detail-readonly" role="status">{provider.readOnlyReason ?? "当前 Provider 只能查看。"}</p> : null}
            {deleteError && !deletingModel ? (
              <section className="provider-detail-model-error" role="alert" aria-live="assertive">
                <div><strong>{deleteError.code === "models-hash-conflict" || deleteError.code === "config-hash-conflict" ? "配置冲突" : "无法删除 Model"}</strong><p>{deleteError.message}</p><p>{deleteError.action}</p></div>
                {(deleteError.code === "models-hash-conflict" || deleteError.code === "config-hash-conflict") ? <Button type="button" variant="secondary" onClick={() => void reload()}>重新读取</Button> : null}
              </section>
            ) : null}
            {providerDeleteError && !deletingProvider ? (
              <section className="provider-detail-model-error" role="alert" aria-live="assertive">
                <div><strong>无法删除 Provider</strong><p>{providerDeleteError.message}</p><p>{providerDeleteError.action}</p></div>
                {(providerDeleteError.code === "models-hash-conflict" || providerDeleteError.code === "config-hash-conflict") ? <Button type="button" variant="secondary" onClick={() => void reload()}>重新读取</Button> : null}
              </section>
            ) : null}
            <section className="provider-detail-search-row" aria-label="Model 操作">
              <SearchInput name="model-search" aria-label="搜索 Model ID" value={modelSearch} onChange={(event) => setModelSearch(event.target.value)} placeholder="搜索 Model ID…" />
              <Button type="button" disabled={!canManageModels} onClick={() => openModelEditor({ mode: "create" })} title={!canManageModels ? "当前 Provider 只读或配置不可写" : undefined}>新增模型</Button>
            </section>
            <section className="provider-detail-summary" aria-label="Provider 摘要">
              <div><span>默认协议</span><strong>{provider.defaultApi ?? "由模型指定"}</strong></div>
              <div><span>认证</span><strong>{authSummary}</strong></div>
              <div><span>模型</span><strong>{provider.modelCount}</strong></div>
              <div><span>状态</span><StatusIndicator tone={provider.editable ? "success" : "warning"}>{provider.editable ? "正常" : "只读"}</StatusIndicator></div>
            </section>
            <section className="provider-detail-models" aria-labelledby="provider-detail-models-title">
              <h2 id="provider-detail-models-title" className="provider-detail-visually-hidden">模型</h2>
              <table>
                <colgroup>
                  <col className="provider-detail-models__identity" /><col className="provider-detail-models__api" /><col className="provider-detail-models__source" /><col className="provider-detail-models__input" /><col className="provider-detail-models__context" /><col className="provider-detail-models__max" /><col className="provider-detail-models__references" /><col className="provider-detail-models__test" /><col className="provider-detail-models__actions" />
                </colgroup>
                <thead><tr><th scope="col">名称 / Model ID</th><th scope="col">有效协议</th><th scope="col">来源</th><th scope="col">能力</th><th scope="col">Context</th><th scope="col">Max Tokens</th><th scope="col">引用</th><th scope="col">最近测试</th><th scope="col">操作</th></tr></thead>
                <tbody>
                  {models.length === 0 ? (
                    <tr><td className="provider-detail-models-empty" colSpan={9}>{provider.models.length === 0 ? "尚未配置 Model definition。" : "没有匹配的 Model definition"}</td></tr>
                  ) : models.map((model) => {
                    const status = modelStatusView(model);
                    const sourceLabel = model.apiSource === "provider" ? "继承 Provider" : model.apiSource === "model" ? "模型指定" : "未配置";
                    const recentResult = modelTest.result?.providerId === provider.id && modelTest.result.modelId === model.id ? modelTest.result : null;
                    const testable = isModelTestable(provider, model, targetWritable);
                    const active = modelTest.isActive(provider.id, model.id);
                    const busy = modelTest.isBusy(provider.id, model.id);
                    return (
                      <tr key={model.id} className={model.status === "read-only" ? "provider-detail-model-row--readonly" : undefined}>
                        <td><div className="provider-detail-model-cell"><strong>{model.name ?? "未命名模型"}</strong><code>{model.id}</code><span className={`provider-detail-model-status provider-detail-model-status--${status.tone}`}>{status.label}</span></div></td>
                        <td>{model.effectiveApi ?? "未配置"}</td>
                        <td title={sourceLabel}>{sourceLabel}</td>
                        <td>{model.input.length ? model.input.map((input) => input === "text" ? "Text" : input === "image" ? "Image" : "不支持").join(" · ") : "未配置"}</td>
                        <td>{formatNumber(model.contextWindow)}</td><td>{formatNumber(model.maxTokens)}</td><td>{model.referenceCount}</td><td>{active ? <StatusIndicator tone="warning">测试中…</StatusIndicator> : recentResult ? (recentResult.success ? `${recentResult.latencyMs} ms` : recentResult.message) : "—"}</td>
                        <td><div className="provider-detail-model-actions">
                          <Button type="button" variant="secondary" className="provider-detail-model-action" aria-label={`Model 操作 ${model.id}`} aria-expanded={openModelActions === model.id} title="Model 操作" onClick={() => setOpenModelActions((current) => current === model.id ? null : model.id)}><MoreHorizontal aria-hidden="true" size={18} /></Button>
                          {openModelActions === model.id ? <div className="provider-detail-model-menu" role="menu">
                            <Button type="button" variant="secondary" role="menuitem" disabled={active ? false : !modelTest.settingsReady || !testable || busy} title={!active && !modelTest.settingsReady ? "正在读取设置" : !active && !testable ? "当前 Model definition 不满足测试条件" : !active && busy ? "已有模型测试正在进行" : undefined} onClick={() => { setOpenModelActions(null); if (active) modelTest.cancel(); else modelTest.start(provider.id, model.id); }}><SquareTerminal aria-hidden="true" size={15} />{active ? "取消测试" : "测试模型"}</Button>
                            {model.editable ? (
                              <>
                                <Button type="button" variant="secondary" role="menuitem" onClick={() => { setOpenModelActions(null); openModelEditor({ mode: "edit", model }); }}><Pencil aria-hidden="true" size={15} />编辑</Button>
                                {model.status === "normal" ? <Button type="button" variant="secondary" role="menuitem" onClick={() => { setOpenModelActions(null); openModelEditor({ mode: "create", model, copy: true }); }}><Copy aria-hidden="true" size={15} />复制</Button> : null}
                                <Button type="button" variant="secondary" role="menuitem" className="provider-detail-model-menu__danger" onClick={() => { setOpenModelActions(null); setDeleteError(null); if (!openedModelsHash || !openedConfigHash) return; setDeleteHashes({ models: openedModelsHash, config: openedConfigHash }); setDeletingModel(model); }}><Trash2 aria-hidden="true" size={15} />删除</Button>
                              </>
                            ) : <Button type="button" variant="secondary" role="menuitem" onClick={() => { setOpenModelActions(null); openModelEditor({ mode: "view", model }); }}><Info aria-hidden="true" size={15} />查看</Button>}
                          </div> : null}
                          {!model.editable ? <span className="provider-detail-model-readonly-label">只读</span> : null}
                        </div></td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </section>
            <section className="provider-detail-latest-test" aria-label="最近测试" aria-live="polite">
              {modelTest.running ? (
                <>
                  <StatusIndicator tone="warning">测试中…</StatusIndicator>
                  <span>{modelTest.activeProviderId && modelTest.activeModelId ? `${modelTest.activeProviderId}/${modelTest.activeModelId}` : "—"}</span>
                  <span>{activeModel?.effectiveApi ?? "—"}</span>
                  <span>请求进行中</span>
                  <Button type="button" variant="secondary" className="provider-detail-latest-test__cancel" aria-label={`取消测试 ${modelTest.activeProviderId ?? "模型"}/${modelTest.activeModelId ?? ""}`} onClick={() => modelTest.cancel()}>取消测试</Button>
                </>
              ) : latestTerminal ? (
                <>
                  <StatusIndicator tone={latestTerminal.errorCode === "cancelled" ? "warning" : "danger"}>{latestTerminal.message}</StatusIndicator>
                  <span>{latestTerminal.providerId}/{latestTerminal.modelId}</span>
                  <span>{latestModel?.effectiveApi ?? "—"}</span>
                  <span>—</span>
                  <span>—</span>
                  <span>—</span>
                </>
              ) : latestResult ? (
                <>
                  <StatusIndicator tone={latestResult.success ? "success" : latestResult.errorCode === "cancelled" ? "warning" : "danger"}>{latestResult.message}</StatusIndicator>
                  <span>{latestResult.providerId}/{latestResult.modelId}</span>
                  <span>{latestResult.protocol}</span>
                  <span>{latestEndpoint.kind === "available" ? latestEndpoint.value : "—"}</span>
                  <span>{latestResult.latencyMs} ms</span>
                  <span>{latestResult.status ? `HTTP ${latestResult.status}` : "—"}</span>
                </>
              ) : <><StatusIndicator tone="neutral">暂无测试结果</StatusIndicator><span>保存后的 Model 测试结果会显示在这里。</span></>}
            </section>
            {modelTest.costNoticeDialog}
            {deletingModel ? <ConfirmDialog title="删除模型？" confirmLabel="删除模型" confirmDisabled={Boolean(modelDeleteBlockedReason || isHashConflict(deleteError))} onCancel={() => { setDeletingModel(null); setDeleteHashes(null); setDeleteError(null); }} onConfirm={() => void deleteModel()}><DeletionImpact objectLabel={`${provider.id}/${deletingModel.id}`} includedModels={[deletingModel.id]} roleReferences={modelRoleReferences} otherReferences={modelOtherReferences} blockedReason={modelDeleteBlockedReason} crossFileTransaction={modelRoleReferences.length > 0} onOpenTargetDirectory={canOpenModelTargetDirectory ? openTargetDirectory : undefined} conflictError={isHashConflict(deleteError) ? deleteError : null} onReload={reloadModelAfterConflict} /></ConfirmDialog> : null}
            {deletingProvider ? <ConfirmDialog title="删除 Provider？" confirmLabel="删除 Provider" confirmDisabled={Boolean(providerDeleteBlockedReason || isHashConflict(providerDeleteError))} onCancel={() => { setDeletingProvider(false); setDeleteHashes(null); setProviderDeleteError(null); }} onConfirm={() => void deleteProvider()}><DeletionImpact objectLabel={provider.id} includedModels={provider.models.map((model) => model.id)} roleReferences={providerRoleReferences} otherReferences={providerOtherReferences} blockedReason={providerDeleteBlockedReason} crossFileTransaction={providerRoleReferences.length > 0} onOpenTargetDirectory={canOpenProviderTargetDirectory ? openTargetDirectory : undefined} conflictError={isHashConflict(providerDeleteError) ? providerDeleteError : null} onReload={reloadProviderAfterConflict} /></ConfirmDialog> : null}
            {editing && editingModelsHash ? <ProviderEditDialog provider={provider} openedModelsHash={editingModelsHash} onDismiss={dismissProviderEditor} onReload={reload} onSaved={async () => reload()} /> : null}
            {modelEditor && modelEditorModelsHash ? <ModelCreateSheet key={`${modelEditor.mode}-${modelEditor.model?.id ?? "new"}-${modelEditor.copy ? "copy" : "edit"}`} provider={provider} targetWritable={targetWritable} openedModelsHash={modelEditorModelsHash} mode={modelEditor.mode} model={modelEditor.model} copy={modelEditor.copy} onDismiss={dismissModelEditor} onReload={reload} onSaved={saveModel} /> : null}
          </>
        )}
      </main>
    </MainShell>
  );
}

function modelStatusView(model: OverviewModel): { label: string; tone: "success" | "warning" | "danger" } {
  if (model.status === "read-only" || !model.editable) return { label: "只读", tone: "danger" };
  if (model.status === "incomplete" || !model.complete) return { label: "配置不完整", tone: "warning" };
  return { label: "正常", tone: "success" };
}

function formatNumber(value: number | null): string {
  return value === null ? "未配置" : new Intl.NumberFormat("zh-CN").format(value);
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
  const hydrateModelTest = useModelTestStore((state) => state.hydrate);
  const prepareModelTestHydration = useModelTestStore((state) => state.prepareHydration);

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

  useEffect(() => {
    prepareModelTestHydration();
    let active = true;
    let timer: number | undefined;
    let failureNotified = false;
    const schedule = () => {
      if (!active || timer !== undefined) return;
      timer = window.setTimeout(() => {
        timer = undefined;
        void syncModelTestState();
      }, MODEL_TEST_STATE_POLL_MS);
    };
    async function syncModelTestState() {
      const generation = useModelTestStore.getState().generation;
      try {
        const state = await client.getModelTestState();
        if (!active) return;
        failureNotified = false;
        hydrateModelTest(state, generation);
        if (state.running) schedule();
      } catch (cause: unknown) {
        if (!active) return;
        if (!failureNotified) {
          const appError = asAppError(cause, "无法读取模型测试状态");
          toast.error(appError.message, { description: appError.action });
          failureNotified = true;
        }
        schedule();
      }
    }
    void syncModelTestState();
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [client, hydrateModelTest, prepareModelTestHydration]);

  return (
    <div className="window">
      <Routes>
        <Route path="/" element={<SetupPage />} />
        <Route path="/setup" element={<SetupPage />} />
        <Route path="/overview" element={<OverviewPage />} />
        <Route path="/providers" element={<ProvidersPage />} />
        <Route path="/providers/:providerId" element={<ProviderDetailPage />} />
        <Route path="/roles" element={<ModelRolesPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Routes>
      <Toaster position="bottom-right" richColors />
    </div>
  );
}
