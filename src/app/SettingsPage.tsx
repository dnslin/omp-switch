import { useCallback, useEffect, useState } from "react";
import { useBlocker, useBeforeUnload } from "react-router";
import { toast } from "sonner";

import { Button, ConfirmDialog, PageTitle } from "../components/ui";
import { RedetectionLoader } from "../components/redetection-loader";
import {
  asAppError,
  useTauriClient,
  type AppError,
  type RuntimeInfo,
  type SettingsDirectories,
  type StartupState,
  type Theme,
  type UiSettings,
} from "../lib/tauri-client";
import { modelSelectionFields, useUiSettings } from "../store/ui-settings";
import { MainShell } from "./MainShell";
import { startupShellStatus, type RowStatus } from "./omp-presentation";

const REDETECT_MINIMUM_DURATION_MS = 1200;

type ReadyState = Extract<StartupState, { kind: "omp-ready" }>;

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

type SettingsPageProps = {
  getTargetPresentation(target: ReadyState["targetConfiguration"], confirmingSwitch: boolean): TargetPresentation;
};

type SettingsCandidate = { state: ReadyState; useSystemPath: boolean; presentation: TargetPresentation };

function settingsStartupError(state: StartupState): AppError {
  if (state.kind === "omp-ready") {
    return { code: "omp-validation-failed", message: "OMP 验证状态无效。", action: "请重新选择或重新检测 OMP。" };
  }
  return {
    code: "omp-validation-failed",
    message: "message" in state ? state.message : "无法验证所选 OMP。",
    action: state.kind === "config-path-failed"
      ? "请检查 OMP 配置路径和权限后重试；OMP Switch 不会猜测目录。"
      : "请检查可执行文件后重试，或选择另一个 OMP。",
  };
}

function settingsTargetError(
  target: ReadyState["targetConfiguration"],
  getTargetPresentation: SettingsPageProps["getTargetPresentation"],
): AppError {
  const presentation = getTargetPresentation(target, true);
  const issue = target.issue ? ` ${target.issue.filePath}: ${target.issue.message}` : "";
  return {
    code: `target-${target.status}`,
    message: `${presentation.title}${issue}`,
    action: `${presentation.description} ${presentation.permissionSummary}`,
  };
}
export function SettingsPage({ getTargetPresentation }: SettingsPageProps) {
  const client = useTauriClient();
  const hydrate = useUiSettings((settings) => settings.hydrate);
  const theme = useUiSettings((settings) => settings.theme);
  const hydrationState = useUiSettings((settings) => settings.hydrationState);
  const hydrationError = useUiSettings((settings) => settings.hydrationError);
  const selection = useUiSettings((settings) => settings.selection);
  const setTheme = useUiSettings((settings) => settings.setTheme);
  const [state, setState] = useState<StartupState>({ kind: "detecting" });
  const [runtimeInfo, setRuntimeInfo] = useState<RuntimeInfo | null>(null);
  const [directories, setDirectories] = useState<SettingsDirectories | null>(null);
  const [candidate, setCandidate] = useState<SettingsCandidate | null>(null);
  const [validationBusy, setValidationBusy] = useState(false);
  const [switchSaving, setSwitchSaving] = useState(false);
  const [themeSaving, setThemeSaving] = useState(false);
  const [redetecting, setRedetecting] = useState(false);
  const [resetOpen, setResetOpen] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [operationError, setOperationError] = useState<AppError | null>(null);
  const dirty = candidate !== null;
  const blocker = useBlocker(dirty);

  useBeforeUnload((event) => {
    if (!dirty) return;
    event.preventDefault();
    event.returnValue = "";
  });

  const reportError = useCallback((cause: unknown, fallback: string) => {
    const error = asAppError(cause, fallback);
    setOperationError(error);
    toast.error(error.message);
    return error;
  }, []);

  const refreshState = useCallback(async () => {
    const [nextState, nextDirectories] = await Promise.all([
      client.getStartupState(),
      client.getSettingsDirectories(),
    ]);
    setState(nextState);
    setDirectories(nextDirectories);
  }, [client]);

  useEffect(() => {
    let active = true;
    void client.getStartupState().then((next) => {
      if (active) setState(next);
    }).catch((cause: unknown) => {
      if (active) reportError(cause, "无法读取启动状态");
    });
    void client.getSettingsDirectories().then((next) => {
      if (active) setDirectories(next);
    }).catch((cause: unknown) => {
      if (active) reportError(cause, "无法读取应用目录");
    });
    void client.getRuntimeInfo().then((next) => {
      if (active) setRuntimeInfo(next);
    }).catch((cause: unknown) => {
      if (active) reportError(cause, "无法读取运行环境");
    });
    return () => { active = false; };
  }, [client, reportError]);

  const applyValidation = useCallback((next: StartupState, useSystemPath: boolean) => {
    if (next.kind !== "omp-ready") {
      const error = settingsStartupError(next);
      setOperationError(error);
      toast.error(error.message);
      return;
    }
    const presentation = getTargetPresentation(next.targetConfiguration, true);
    if (next.targetConfiguration.status === "writable" || next.targetConfiguration.status === "read-only" || next.targetConfiguration.status === "creation-required") {
      setCandidate({ state: next, useSystemPath, presentation });
      setOperationError(null);
      return;
    }
    const error = settingsTargetError(next.targetConfiguration, getTargetPresentation);
    setOperationError(error);
    toast.error(error.message);
  }, [getTargetPresentation]);

  const selectExecutable = useCallback(async () => {
    if (validationBusy || switchSaving) return;
    try {
      const selected = await client.selectOmpExecutable();
      if (!selected) return;
      setValidationBusy(true);
      setOperationError(null);
      applyValidation(await client.validateSelectedOmp(selected), false);
    } catch (cause: unknown) {
      reportError(cause, "无法验证所选 OMP");
    } finally {
      setValidationBusy(false);
    }
  }, [applyValidation, client, reportError, switchSaving, validationBusy]);

  const useSystemPath = useCallback(async () => {
    if (validationBusy || switchSaving) return;
    setValidationBusy(true);
    setOperationError(null);
    try {
      applyValidation(await client.validatePathOmp(), true);
    } catch (cause: unknown) {
      reportError(cause, "无法验证系统 PATH 中的 OMP");
    } finally {
      setValidationBusy(false);
    }
  }, [applyValidation, client, reportError, switchSaving, validationBusy]);

  const confirmCandidate = useCallback(async () => {
    if (!candidate || switchSaving) return;
    const selected = candidate;
    setSwitchSaving(true);
    setOperationError(null);
    try {
      let saved: UiSettings;
      if (selected.state.targetConfiguration.status === "creation-required") {
        const initialized = await client.initializeTargetConfiguration(selected.state.executablePath, {
          createPaths: selected.state.targetConfiguration.createPaths,
          discoveryToken: selected.state.targetConfiguration.discoveryToken,
        });
        if (initialized.kind !== "omp-ready" || initialized.targetConfiguration.status === "creation-required") {
          setState(initialized);
          throw { code: "target-initialization-incomplete", message: "Target configuration 创建未完成。", action: "请重新检测并确认最新的创建清单。" } satisfies AppError;
        }
        saved = await client.getUiSettings();
      } else if (selected.useSystemPath) {
        saved = await client.confirmPathOmp(selected.state.executablePath);
      } else {
        await client.confirmSelectedOmp(selected.state.executablePath);
        saved = await client.getUiSettings();
      }
      hydrate(saved);
      setCandidate(null);
      try {
        await refreshState();
        const refreshed = await client.getOverviewLoad();
        if (refreshed.startupState.kind === "omp-ready") setState(refreshed.startupState);
        if (refreshed.error) throw refreshed.error;
        if (!refreshed.overview) {
          throw { code: "overview-read-empty", message: "OMP 已切换，但无法读取新 Target configuration。", action: "请重新检测 OMP 后重试。" } satisfies AppError;
        }
      } catch (cause: unknown) {
        throw asAppError(cause, "OMP 已切换，但无法读取新 Target configuration");
      }
      toast.success(selected.useSystemPath ? "已切换为系统 PATH 中的 OMP" : "OMP 已切换");
    } catch (cause: unknown) {
      reportError(cause, "无法保存 OMP 选择");
    } finally {
      setSwitchSaving(false);
    }
  }, [candidate, client, hydrate, refreshState, reportError, switchSaving]);

  const redetect = useCallback(async () => {
    if (redetecting || validationBusy || switchSaving || candidate) return;
    setRedetecting(true);
    setOperationError(null);
    try {
      const [nextState] = await Promise.all([
        client.detectOmp(),
        new Promise<void>((resolve) => window.setTimeout(resolve, REDETECT_MINIMUM_DURATION_MS)),
      ]);
      setState(nextState);
      setDirectories(await client.getSettingsDirectories());
      toast.success("OMP 已重新检测");
    } catch (cause: unknown) {
      reportError(cause, "无法重新检测 OMP");
    } finally {
      setRedetecting(false);
    }
  }, [candidate, client, redetecting, reportError, switchSaving, validationBusy]);

  const saveTheme = useCallback(async (nextTheme: Theme) => {
    if (hydrationState !== "ready" || themeSaving || nextTheme === theme) return;
    const previousTheme = theme;
    setTheme(nextTheme);
    setThemeSaving(true);
    setOperationError(null);
    try {
      const saved = await client.saveUiSettings({ theme: nextTheme, ...modelSelectionFields(selection) });
      hydrate(saved);
      toast.success("外观设置已保存");
    } catch (cause: unknown) {
      setTheme(previousTheme);
      reportError(cause, "无法保存外观设置");
    } finally {
      setThemeSaving(false);
    }
  }, [client, hydrate, hydrationState, reportError, selection, setTheme, theme, themeSaving]);

  const openDirectory = useCallback(async (operation: () => Promise<void>, fallback: string) => {
    try {
      await operation();
    } catch (cause: unknown) {
      reportError(cause, fallback);
    }
  }, [reportError]);

  const resetSettings = useCallback(async () => {
    if (resetting) return;
    setResetting(true);
    setOperationError(null);
    try {
      hydrate(await client.resetUiSettings());
      setResetOpen(false);
      try {
        await refreshState();
      } catch (cause: unknown) {
        reportError(cause, "默认设置已恢复，但无法刷新 OMP 状态");
      }
      toast.success("应用默认设置已恢复");
    } catch (cause: unknown) {
      reportError(cause, "无法恢复应用默认设置");
    } finally {
      setResetting(false);
    }
  }, [client, hydrate, reportError, refreshState, resetting]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "s") return;
      if (!candidate || blocker.state === "blocked") return;
      event.preventDefault();
      void confirmCandidate();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [blocker.state, candidate, confirmCandidate]);

  const ready = state.kind === "omp-ready";
  const targetPath = directories?.targetConfiguration ?? (ready ? state.targetConfiguration.resolvedPath ?? state.targetConfiguration.path : "未确认");
  const backupPath = directories?.targetBackup ?? "确认 Target configuration 后可用";
  const executablePath = ready ? state.executablePath : "未确认";
  const version = ready ? state.version : "—";
  const runtime = runtimeInfo ? `${runtimeInfo.platform} · ${runtimeInfo.architecture}` : "未知 · —";
  const startupError = state.kind !== "omp-ready" && state.kind !== "detecting" ? settingsStartupError(state) : null;
  const visibleError = operationError ?? hydrationError ?? startupError;
  const candidateNeedsInitialization = candidate?.state.targetConfiguration.status === "creation-required";

  return (
    <MainShell status={startupShellStatus(state)} contentClassName="page-content--settings">
      <div className="settings-page" aria-busy={redetecting || validationBusy || switchSaving || resetting}>
        <PageTitle title="设置" description="管理 OMP 路径、外观和应用目录。" />
        {visibleError ? (
          <section className="settings-error" role="alert" aria-live="assertive">
            <strong>{visibleError.message}</strong>
            <p>{visibleError.action}</p>
          </section>
        ) : null}
        <div id="omp-settings" className="settings-panel">
          <h2>OMP</h2>
          <div className="settings-row settings-row--executable">
            <span className="settings-row__label">OMP 可执行文件</span>
            <code>{executablePath}</code>
            <div className="settings-row__actions">
              <Button type="button" variant="secondary" disabled={validationBusy || switchSaving} onClick={() => void selectExecutable()}>重新选择</Button>
              <Button type="button" disabled={validationBusy || switchSaving} onClick={() => void useSystemPath()}>使用系统 PATH</Button>
            </div>
          </div>
          <div className="settings-row">
            <span className="settings-row__label">版本</span>
            <code>{version}</code>
          </div>
          <div className="settings-row">
            <span className="settings-row__label">权威配置目录</span>
            <code title={targetPath}>{targetPath}</code>
            <div className="settings-row__actions">
              <Button type="button" variant="secondary" disabled={redetecting || validationBusy || switchSaving} onClick={() => void redetect()}>重新检测</Button>
            </div>
          </div>
          <h2>外观</h2>
          <div className="settings-row settings-row--theme">
            <span className="settings-row__label">外观模式</span>
            <div className="settings-theme-options" role="group" aria-label="外观模式">
              {([["system", "跟随系统"], ["light", "浅色"], ["dark", "深色"]] as const).map(([value, label]) => (
                <Button key={value} type="button" variant="secondary" className={`settings-theme-option ${theme === value ? "settings-theme-option--selected" : ""}`} disabled={hydrationState !== "ready" || themeSaving} aria-pressed={theme === value} onClick={() => void saveTheme(value)}>{label}</Button>
              ))}
            </div>
            <span className="settings-row__hint">立即生效并自动保存。</span>
          </div>
          <h2>目录</h2>
          <div className="settings-row">
            <span className="settings-row__label">OMP 配置目录</span>
            <code title={targetPath}>{targetPath}</code>
            <div className="settings-row__actions"><Button type="button" variant="secondary" aria-label="打开 OMP 配置目录" onClick={() => void openDirectory(client.openCurrentTargetConfigurationDirectory, "无法打开配置目录")}>打开</Button></div>
          </div>
          <div className="settings-row">
            <span className="settings-row__label">应用配置目录</span>
            <code title={directories?.applicationConfiguration}>{directories?.applicationConfiguration ?? "正在读取…"}</code>
            <div className="settings-row__actions"><Button type="button" variant="secondary" aria-label="打开应用配置目录" onClick={() => void openDirectory(client.openApplicationConfigurationDirectory, "无法打开应用配置目录")}>打开</Button></div>
          </div>
          <div className="settings-row">
            <span className="settings-row__label">应用日志目录</span>
            <code title={directories?.applicationLog}>{directories?.applicationLog ?? "正在读取…"}</code>
            <div className="settings-row__actions"><Button type="button" variant="secondary" aria-label="打开应用日志目录" onClick={() => void openDirectory(client.openApplicationLogDirectory, "无法打开应用日志目录")}>打开</Button></div>
          </div>
          <div className="settings-row">
            <span className="settings-row__label">备份目录</span>
            <code title={backupPath}>{backupPath}</code>
            <div className="settings-row__actions"><Button type="button" variant="secondary" aria-label="打开备份目录" onClick={() => void openDirectory(client.openTargetBackupDirectory, "无法打开备份目录")}>打开</Button></div>
          </div>
          <h2>应用信息</h2>
          <div className="settings-row"><span className="settings-row__label">应用名称</span><span>OMP Switch</span></div>
          <div className="settings-row"><span className="settings-row__label">版本</span><code>0.1.0</code></div>
          <div className="settings-row"><span className="settings-row__label">运行环境</span><span>{runtime}</span></div>
          <div className="settings-row"><span className="settings-row__label">日志</span><span /><div className="settings-row__actions"><Button type="button" variant="secondary" aria-label="查看日志" onClick={() => void openDirectory(client.openApplicationLogDirectory, "无法打开应用日志目录")}>查看日志</Button></div></div>
          <h2>恢复默认设置</h2>
          <div className="settings-row settings-row--reset">
            <span className="settings-row__label">恢复默认设置</span>
            <span>只恢复主题、OMP 路径和当前选择项；不会删除 Provider、模型、角色、API Key 或备份。</span>
            <div className="settings-row__actions"><Button type="button" variant="secondary" className="settings-danger-button" onClick={() => setResetOpen(true)}>恢复默认设置</Button></div>
          </div>
        </div>
      </div>
      {candidate ? (
        <ConfirmDialog
          title={candidateNeedsInitialization ? "确认创建并切换 OMP" : "确认切换 OMP"}
          cancelLabel="取消"
          confirmLabel={switchSaving ? (candidateNeedsInitialization ? "创建中…" : "保存中…") : (candidateNeedsInitialization ? "确认创建并切换" : "确认切换")}
          confirmDisabled={switchSaving}
          onCancel={() => { if (!switchSaving) setCandidate(null); }}
          onConfirm={() => void confirmCandidate()}
        >
          <p>{candidate.presentation.description}</p>
          <p>将使用 {candidate.useSystemPath ? "系统 PATH 中的 OMP" : "新的 OMP 可执行文件"}。</p>
          <div className="settings-target-change">
            <div><span>当前 Target configuration</span><code>{candidate.state.previousTargetConfiguration ?? targetPath}</code></div>
            <div><span>新的 Target configuration</span><code>{candidate.state.targetConfiguration.resolvedPath ?? candidate.state.targetConfiguration.path}</code></div>
          </div>
          {candidateNeedsInitialization ? (
            <div className="setup-notice">
              <strong>将创建</strong>
              <ul>{candidate.state.targetConfiguration.createPaths.map((path) => <li key={path}><code>{path}</code></li>)}</ul>
            </div>
          ) : null}
          {candidate.state.targetConfiguration.status === "read-only" ? <p>新 Target configuration 只读；确认后进入只读状态，不会写入配置。</p> : null}
          <p>{candidateNeedsInitialization ? "确认后通过可恢复事务创建最小配置，并重新读取新 Target configuration。" : "确认后会重新读取配置，并清除不适用于新 Target 的轻量选择。"}</p>
        </ConfirmDialog>
      ) : null}
      {resetOpen ? (
        <ConfirmDialog
          title="恢复应用默认设置？"
          cancelLabel="取消"
          confirmLabel={resetting ? "恢复中…" : "恢复默认"}
          confirmDisabled={resetting}
          onCancel={() => { if (!resetting) setResetOpen(false); }}
          onConfirm={() => void resetSettings()}
        >
          <p>将恢复主题、OMP 路径和当前选择项。</p>
          <p>不会删除 OMP Provider、模型、角色、API Key 或备份。</p>
        </ConfirmDialog>
      ) : null}
      {blocker.state === "blocked" ? (
        <ConfirmDialog
          title="有未保存的修改"
          cancelLabel="继续编辑"
          confirmLabel="放弃修改"
          onCancel={() => blocker.reset()}
          onConfirm={() => { setCandidate(null); blocker.proceed(); }}
        >
          离开后，这些修改将会丢失。
        </ConfirmDialog>
      ) : null}
      {redetecting ? (
        <div className="redetect-overlay" role="status" aria-live="polite">
          <div className="redetect-overlay__content"><RedetectionLoader /><strong>正在重新检测 OMP</strong></div>
        </div>
      ) : null}
    </MainShell>
  );
}
