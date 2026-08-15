import { CircleAlert, CircleCheck, Info } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { toast } from "sonner";

import { Button } from "../components/ui";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { asAppError, useTauriClient, type AppError, type OverviewDto, type OverviewModel, type OverviewProvider, type TauriClient, type UiSettingsUpdate } from "../lib/tauri-client";
import { modelSelectionFields, useUiSettings, type ModelSelection } from "../store/ui-settings";
import { MainShell } from "./MainShell";
import { fileStatusView } from "./omp-presentation";
import { useOverviewLoad } from "./overview-load";

type OverviewError = Pick<AppError, "message" | "action">;

const overviewLoadCopy = {
  missingOverview: {
    code: "overview-missing-data",
    message: "OMP 没有返回概览数据。",
    action: "请重新读取；如果问题持续，请查看脱敏日志。",
  },
  requestFailure: "无法读取概览",
};

type InitialOverviewSelection = {
  selection: ModelSelection;
  staleSavedSelection: boolean;
};

const overviewSettingsSaveQueues = new WeakMap<TauriClient, Promise<void>>();

function enqueueOverviewSettingsSave(client: TauriClient, settings: UiSettingsUpdate, isActive: () => boolean) {
  const previous = overviewSettingsSaveQueues.get(client) ?? Promise.resolve();
  const queued = previous.then(async () => {
    try {
      await client.saveUiSettings(settings);
    } catch (cause: unknown) {
      if (!isActive()) return;
      const appError = asAppError(cause, "无法保存快速测试选择");
      toast.error(appError.message, { description: appError.action });
    }
  });
  overviewSettingsSaveQueues.set(client, queued);
}

function preferredOverviewModel(models: readonly OverviewModel[]): OverviewModel | null {
  return models.find((model) => model.complete && model.editable)
    ?? models.find((model) => model.editable)
    ?? models.find((model) => model.complete)
    ?? models[0]
    ?? null;
}

function defaultOverviewSelection(data: OverviewDto): InitialOverviewSelection {
  const preferredModel = preferredOverviewModel(data.models);
  if (preferredModel) {
    const provider = data.providers.find((candidate) => candidate.id === preferredModel.providerId);
    const model = provider?.models.find((candidate) => candidate.id === preferredModel.id);
    if (provider && model) {
      return { selection: { kind: "model", providerId: provider.id, modelId: model.id }, staleSavedSelection: false };
    }
  }
  const provider = data.providers[0];
  return {
    selection: provider ? { kind: "provider", providerId: provider.id } : { kind: "none" },
    staleSavedSelection: false,
  };
}

function resolveInitialOverviewSelection(
  data: OverviewDto,
  hydrationState: "loading" | "ready" | "error",
  storedSelection: ModelSelection,
  savedSelectionInvalid: boolean,
): InitialOverviewSelection {
  if (hydrationState !== "ready") return defaultOverviewSelection(data);
  if (savedSelectionInvalid) return { selection: { kind: "none" }, staleSavedSelection: true };

  switch (storedSelection.kind) {
    case "none":
      return defaultOverviewSelection(data);
    case "provider": {
      const provider = data.providers.find((candidate) => candidate.id === storedSelection.providerId);
      return provider
        ? { selection: { kind: "provider", providerId: provider.id }, staleSavedSelection: false }
        : { selection: { kind: "none" }, staleSavedSelection: true };
    }
    case "model": {
      const provider = data.providers.find((candidate) => candidate.id === storedSelection.providerId);
      if (!provider) return { selection: { kind: "none" }, staleSavedSelection: true };
      const model = provider.models.find((candidate) => candidate.id === storedSelection.modelId);
      return model
        ? { selection: { kind: "model", providerId: provider.id, modelId: model.id }, staleSavedSelection: false }
        : { selection: { kind: "provider", providerId: provider.id }, staleSavedSelection: true };
    }
  }
}

function sameModelSelection(left: ModelSelection, right: ModelSelection) {
  if (left.kind !== right.kind) return false;
  if (left.kind === "none" || right.kind === "none") return true;
  if (left.providerId !== right.providerId) return false;
  return left.kind === "provider" || right.kind === "provider" || left.modelId === right.modelId;
}

export function OverviewPage() {
  const client = useTauriClient();
  const hydrationState = useUiSettings((state) => state.hydrationState);
  const { data, startupState, error, loading, reload, shellStatus } = useOverviewLoad(overviewLoadCopy);

  async function openTargetDirectory() {
    if (startupState?.kind !== "omp-ready") return;
    try {
      await client.openTargetConfigurationDirectory(startupState.executablePath);
    } catch (cause: unknown) {
      const appError = asAppError(cause, "无法打开配置目录");
      toast.error(appError.message, { description: appError.action });
    }
  }

  const pageLoading = loading || hydrationState === "loading";
  const pageClass = pageLoading ? "overview-page--loading" : error || !data ? "overview-page--error" : `overview-page--${data.state}`;
  const openDirectory = startupState?.kind === "omp-ready" ? openTargetDirectory : null;
  return (
    <MainShell status={shellStatus}>
      <div className={`overview-page ${pageClass}`} aria-busy={pageLoading}>
        <OverviewPageHeader />
        {pageLoading ? <OverviewLoadingBody /> : error ? <OverviewErrorBody error={error} onReload={reload} onOpenTargetDirectory={openDirectory} /> : data ? <OverviewContentBody data={data} /> : <OverviewErrorBody error={overviewLoadCopy.missingOverview} onReload={reload} onOpenTargetDirectory={openDirectory} />}
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

function OverviewSkeletonSlots() {
  return (
    <>
      <div className="overview-skeleton overview-skeleton--sync" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--environment" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--metrics" aria-hidden="true" />
      <div className="overview-skeleton overview-skeleton--test" aria-hidden="true" />
    </>
  );
}

function OverviewLoadingBody() {
  return (
    <div className="overview-loading" role="status" aria-label="正在读取配置" aria-live="polite">
      <strong>正在读取配置…</strong>
      <OverviewSkeletonSlots />
    </div>
  );
}

function OverviewErrorBody({ error, onReload, onOpenTargetDirectory }: { error: OverviewError; onReload: () => Promise<AppError | null>; onOpenTargetDirectory: (() => Promise<void>) | null }) {
  return (
    <div className="overview-error-scaffold">
      <section className="overview-error-card" role="alert" aria-live="assertive">
        <CircleAlert aria-hidden="true" />
        <div>
          <h2>无法读取概览</h2>
          <p>{error.message}</p>
          <p className="overview-state-detail">{error.action}</p>
        </div>
        <div className="overview-error-actions">
          {onOpenTargetDirectory ? <Button variant="secondary" onClick={() => void onOpenTargetDirectory()}>打开配置目录</Button> : null}
          <Button variant="secondary" onClick={() => void onReload()}>重新读取</Button>
        </div>
      </section>
      <OverviewSkeletonSlots />
    </div>
  );
}

function OverviewContentBody({ data }: { data: OverviewDto }) {
  const client = useTauriClient();
  const hydrationState = useUiSettings((state) => state.hydrationState);
  const storedSelection = useUiSettings((state) => state.selection);
  const savedSelectionInvalid = useUiSettings((state) => state.savedSelectionInvalid);
  const setStoredSelection = useUiSettings((state) => state.setSelection);
  const isActive = useRef(true);
  const staleSavedSelectionHandled = useRef(false);
  const [initialSelection] = useState(() => resolveInitialOverviewSelection(data, hydrationState, storedSelection, savedSelectionInvalid));
  const [selection, setSelection] = useState<ModelSelection>(initialSelection.selection);
  useEffect(() => {
    isActive.current = true;
    return () => { isActive.current = false; };
  }, []);
  const enqueueSelectionSave = useCallback((nextSelection: ModelSelection) => {
    const { theme, costNoticeAccepted } = useUiSettings.getState();
    const settings = { theme, ...modelSelectionFields(nextSelection), costNoticeAccepted };
    enqueueOverviewSettingsSave(client, settings, () => isActive.current);
  }, [client]);

  useEffect(() => {
    setStoredSelection(initialSelection.selection);
    if (!initialSelection.staleSavedSelection || staleSavedSelectionHandled.current) return;
    staleSavedSelectionHandled.current = true;
    enqueueSelectionSave(initialSelection.selection);
    toast.warning("之前选择的模型已不存在，请重新选择。");
  }, [enqueueSelectionSave, initialSelection, setStoredSelection]);

  const selectedProvider = selection.kind === "none"
    ? null
    : data.providers.find((provider) => provider.id === selection.providerId) ?? null;
  const selectedModel = selection.kind === "model"
    ? selectedProvider?.models.find((model) => model.id === selection.modelId) ?? null
    : null;

  function commitSelection(nextSelection: ModelSelection) {
    if (sameModelSelection(selection, nextSelection)) return;
    setSelection(nextSelection);
    setStoredSelection(nextSelection);
    if (hydrationState === "ready") enqueueSelectionSave(nextSelection);
  }

  function handleProviderChange(providerId: string) {
    const nextProvider = data.providers.find((provider) => provider.id === providerId);
    if (!nextProvider) return;
    commitSelection({ kind: "provider", providerId: nextProvider.id });
  }

  function handleModelChange(modelId: string) {
    if (!selectedProvider) return;
    const nextModel = selectedProvider.models.find((model) => model.id === modelId);
    if (!nextModel) return;
    commitSelection({ kind: "model", providerId: selectedProvider.id, modelId: nextModel.id });
  }
  return (
    <>
      <OverviewSyncStrip data={data} />
      <OverviewEnvironment data={data} />
      <OverviewMetrics data={data} />
      {data.state === "empty" || data.state === "read-only" ? <OverviewStateBanner data={data} /> : null}
      <div className="overview-test-area">
        <QuickTestPanel providers={data.providers} provider={selectedProvider} model={selectedModel} onProviderChange={handleProviderChange} onModelChange={handleModelChange} />
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
  const synced = file.contentHash !== null && file.status === "normal";
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
  const configurationCreationRequired = empty && (
    data.targetConfiguration.status === "creation-required"
      || data.files.models.status === "missing"
      || data.files.config.status === "missing"
  );
  const providerManagementReadOnly = !empty
    && data.targetConfiguration.status === "writable"
    && data.providers.length > 0
    && data.providers.every((provider) => !provider.editable);
  const title = configurationCreationRequired
    ? "还没有可读取的规范配置文件"
    : empty
      ? "还没有可管理的自定义 Provider"
      : providerManagementReadOnly
        ? "没有可编辑的自定义 Provider"
        : "配置只读";
  const actionLabel = configurationCreationRequired ? "完成首次设置" : empty ? "新增 Provider" : "查看 Providers";
  const actionPath = configurationCreationRequired ? "/setup" : "/providers";
  return (
    <section className={`overview-state-banner overview-state-banner--${empty ? "empty" : "readonly"}`} aria-live="polite">
      {empty ? <CircleCheck aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}
      <div>
        <strong>{title}</strong>
        <p>{empty ? (data.emptyReason ?? "创建一个 Provider，并同时配置它的第一个模型。") : (data.readOnlyReason ?? "当前配置只能查看；OMP Switch 不会修改配置文件。")}</p>
        {data.nextAction ? <p className="overview-state-detail">{data.nextAction}</p> : null}
      </div>
      <Button asChild variant={empty ? "primary" : "secondary"}>
        <Link to={actionPath}>{actionLabel}</Link>
      </Button>
    </section>
  );
}

function QuickTestPanel({
  providers,
  provider,
  model,
  onProviderChange,
  onModelChange,
}: {
  providers: readonly OverviewProvider[];
  provider: OverviewProvider | null;
  model: OverviewModel | null;
  onProviderChange(value: string): void;
  onModelChange(value: string): void;
}) {
  const endpoint = provider && model ? modelEndpoint(provider, model) : { kind: "not-configured" as const };
  const finalAddress = endpoint.kind === "available" ? endpoint.value : endpoint.kind === "invalid" ? endpoint.reason : "—";
  const protocol = model?.effectiveApi ? `${model.effectiveApi}  ·  ${model.apiSource === "provider" ? "Provider 默认值" : "模型指定"}` : "—";
  const capabilities = model
    ? [...model.input.map((input) => input === "text" ? "Text" : input === "image" ? "Image" : "Unsupported"), ...(model.reasoning === true ? ["Reasoning"] : [])].join("  ·  ") || "—"
    : "—";
  return (
    <section className="overview-panel overview-quick-test" aria-label="快速测试">
      <h2>快速测试</h2>
      <OverviewSelectField
        label="Provider"
        value={provider?.id ?? null}
        placeholder={providers.length === 0 ? "暂无 Provider" : "请选择 Provider"}
        options={providers.map((option) => ({ value: option.id, label: option.id }))}
        disabled={providers.length === 0}
        onValueChange={onProviderChange}
      />
      <OverviewSelectField
        label="模型"
        value={model?.id ?? null}
        placeholder={!provider || provider.models.length === 0 ? "暂无模型" : "请选择模型"}
        options={provider?.models.map((option) => ({ value: option.id, label: option.id })) ?? []}
        disabled={!provider || provider.models.length === 0}
        onValueChange={onModelChange}
      />
      <OverviewField label="有效协议" value={protocol} />
      <OverviewField label="最终地址" value={finalAddress} mono />
      <OverviewField label="能力" value={capabilities} />
      <OverviewField label="Context Window" value={model?.contextWindow ? formatOverviewCount(model.contextWindow) : "—"} />
      <div className="overview-panel-actions"><Button disabled aria-label="测试模型（尚未启用）">测试模型</Button></div>
    </section>
  );
}

function OverviewSelectField({
  label,
  value,
  placeholder,
  options,
  disabled,
  onValueChange,
}: {
  label: string;
  value: string | null;
  placeholder: string;
  options: readonly { value: string; label: string }[];
  disabled: boolean;
  onValueChange(value: string): void;
}) {
  return (
    <div className="overview-field">
      <span>{label}</span>
      <Select value={value ?? ""} onValueChange={onValueChange} disabled={disabled}>
        <SelectTrigger
          aria-label={label}
          className="h-10 min-h-10 w-full min-w-0 px-3.5 text-base text-[var(--color-text-primary)] data-[placeholder]:text-[var(--color-text-disabled)] disabled:cursor-not-allowed disabled:opacity-100 disabled:text-[var(--color-text-disabled)] [&>span]:min-w-0 [&>span]:truncate"
        >
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent position="popper" sideOffset={4} className="max-h-80 w-[var(--radix-select-trigger-width)] overflow-y-auto">
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value} className="min-w-0 text-[var(--color-text-primary)]">
              <span className="block min-w-0 truncate">{option.label}</span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

function OverviewField({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="overview-field">
      <span>{label}</span>
      <div className={`overview-field-control ${mono ? "overview-field-control--mono" : ""}`}>
        <span>{value}</span>
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


function formatOverviewCount(value: number) {
  return new Intl.NumberFormat("zh-CN").format(value);
}

type ModelEndpoint =
  | { kind: "available"; value: string }
  | { kind: "not-configured" }
  | { kind: "invalid"; reason: string };

function modelEndpoint(provider: OverviewProvider, model: OverviewModel): ModelEndpoint {
  if (model.hasBaseUrlOverride) {
    return { kind: "invalid", reason: "模型级 Base URL 覆盖不可安全展示" };
  }
  const base = provider.baseUrl?.trim();
  if (!base || !model.effectiveApi) return { kind: "not-configured" };
  try {
    const endpoint = new URL(base);
    if (endpoint.protocol !== "http:" && endpoint.protocol !== "https:") {
      return { kind: "invalid", reason: "Provider Base URL 必须使用 HTTP(S)" };
    }
    switch (model.effectiveApi) {
      case "openai-completions":
        return { kind: "available", value: appendEndpointPath(endpoint, "chat/completions").toString() };
      case "openai-responses":
        return { kind: "available", value: appendEndpointPath(endpoint, "responses").toString() };
      case "anthropic-messages":
        return { kind: "available", value: appendEndpointPath(endpoint, "v1/messages").toString() };
      case "google-generative-ai": {
        const googleEndpoint = appendEndpointPath(endpoint, `models/${encodeURIComponent(model.id)}:streamGenerateContent`);
        googleEndpoint.searchParams.set("alt", "sse");
        return { kind: "available", value: googleEndpoint.toString() };
      }
      default:
        return { kind: "invalid", reason: "有效协议不受支持" };
    }
  } catch {
    return { kind: "invalid", reason: "Provider Base URL 无效或已脱敏" };
  }
}

function appendEndpointPath(endpoint: URL, suffix: string) {
  const basePath = endpoint.pathname.replace(/\/+$/, "");
  endpoint.pathname = `${basePath}/${suffix}`;
  return endpoint;
}
