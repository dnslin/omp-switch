import { CircleAlert, MoreHorizontal } from "lucide-react";
import { Fragment, useState } from "react";
import { useNavigate } from "react-router";
import { toast } from "sonner";

import { Button, SearchInput, StatusIndicator } from "../components/ui";
import { type AppError, type OverviewDto, type OverviewProvider } from "../lib/tauri-client";
import { MainShell } from "./MainShell";
import { ProviderCreateDialog } from "./ProviderCreateDialog";
import { useOverviewLoad } from "./overview-load";

type ProviderStatus = {
  label: string;
  tone: "success" | "warning" | "danger";
};

const providersLoadCopy = {
  missingOverview: {
    code: "providers-missing-overview",
    message: "OMP 没有返回 Provider 列表数据。",
    action: "请重新读取；如果问题持续，请查看脱敏日志。",
  },
  requestFailure: "无法读取 Providers",
};

function providerStatus(provider: OverviewProvider): ProviderStatus {
  if (provider.editable && provider.models.some((model) => !model.complete)) {
    return { label: "配置不完整", tone: "warning" };
  }
  if (provider.editable) return { label: "正常", tone: "success" };
  switch (provider.classification) {
    case "built-in-override":
      return { label: "内置覆盖 · 只读", tone: "warning" };
    case "advanced":
      return { label: "高级配置 · 只读", tone: "warning" };
    case "unsupported":
      return { label: "不支持 · 只读", tone: "danger" };
    case "unavailable":
      return { label: "清单缺失 · 只读", tone: "warning" };
    case "custom":
      return { label: "只读", tone: "warning" };
  }
}

export function providerAuthSummary(provider: OverviewProvider): string {
  if (provider.authMode === "api-key") return provider.hasApiKey ? "API Key 已配置" : "API Key 未配置";
  if (provider.authMode === "none") return "无认证";
  return "不支持的认证";
}

function matchesProvider(provider: OverviewProvider, query: string): boolean {
  if (!query) return true;
  const fields = [
    provider.id,
    provider.name,
    provider.baseUrl,
    provider.defaultApi,
    provider.classification,
    provider.readOnlyReason,
    providerAuthSummary(provider),
    ...provider.models.flatMap((model) => [
      model.id,
      model.name,
      model.effectiveApi,
      model.readOnlyReason,
      model.input.join(" "),
    ]),
  ];
  return fields.some((field) => field?.toLowerCase().includes(query));
}

export function ProvidersPage() {
  const navigate = useNavigate();
  const { data, error, loading, reload, shellStatus } = useOverviewLoad(providersLoadCopy);
  const [openedModelsHash, setOpenedModelsHash] = useState<string | null>(null);
  const canCreate = data?.state !== "read-only" && Boolean(data?.files.models.contentHash);
  const createTitle = data?.state === "read-only"
    ? data.readOnlyReason ?? "当前 Provider 仅可查看；OMP Switch 不会修改配置文件。"
    : data?.files.models.contentHash
      ? ""
      : "当前 models.yml 没有可用于冲突检查的内容 Hash。";

  const created = async ({ providerId }: { providerId: string }): Promise<AppError | null> => {
    const reloadError = await reload();
    if (reloadError) return reloadError;
    setOpenedModelsHash(null);
    toast.success("Provider 和首个模型已创建");
    navigate(`/providers/${encodeURIComponent(providerId)}`);
    return null;
  };

  return (
    <MainShell status={shellStatus}>
      <main className={`providers-page ${loading ? "providers-page--loading" : ""}`} aria-busy={loading}>
        <header className="providers-header">
          <div className="page-title">
            <h1>Providers</h1>
            <p>管理自定义 API 服务及其模型。</p>
          </div>
          <Button
            type="button"
            disabled={!canCreate}
            disabledAppearance="stable"
            title={createTitle}
            onClick={() => {
              const hash = data?.files.models.contentHash;
              if (hash) setOpenedModelsHash(hash);
            }}
          >
            新增 Provider
          </Button>
        </header>
        {loading ? <ProvidersLoading /> : error ? <ProvidersError error={error} onReload={reload} /> : data ? <ProvidersTable data={data} /> : <ProvidersError error={providersLoadCopy.missingOverview} onReload={reload} />}
      </main>
      {openedModelsHash ? (
        <ProviderCreateDialog
          openedModelsHash={openedModelsHash}
          onDismiss={() => setOpenedModelsHash(null)}
          onReload={reload}
          onCreated={created}
        />
      ) : null}
    </MainShell>
  );
}

function ProvidersLoading() {
  return (
    <div className="providers-loading" role="status" aria-live="polite">
      <span>正在读取 Providers…</span>
      <div className="providers-skeleton providers-skeleton--search" aria-hidden="true" />
      <div className="providers-skeleton providers-skeleton--table" aria-hidden="true" />
    </div>
  );
}

function ProvidersError({ error, onReload }: { error: AppError; onReload: () => Promise<AppError | null> }) {
  return (
    <section className="providers-error" role="alert" aria-live="assertive">
      <CircleAlert aria-hidden="true" />
      <div>
        <h2>无法读取 Providers</h2>
        <p>{error.message}</p>
        <p>{error.action}</p>
      </div>
      <Button type="button" variant="secondary" onClick={() => void onReload()}>重新读取</Button>
    </section>
  );
}

function ProvidersTable({ data }: { data: OverviewDto }) {
  const [query, setQuery] = useState("");
  const normalizedQuery = query.trim().toLowerCase();
  const providers = data.providers.filter((provider) => matchesProvider(provider, normalizedQuery));
  const managementReason = data.readOnlyReason ?? "当前 Provider 仅可查看；OMP Switch 不会修改配置文件。";

  return (
    <>
      {data.state === "read-only" ? (
        <section className="providers-lock-banner" role="status" aria-live="polite">
          <CircleAlert aria-hidden="true" />
          <div><strong>Provider 与模型管理只读</strong><p>{managementReason}</p></div>
        </section>
      ) : null}
      <SearchInput
        className="providers-search"
        type="search"
        name="provider-search"
        aria-label="搜索 Provider"
        placeholder="搜索 Provider..."
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      <div className="providers-table-scroll">
        <table className="providers-table">
          <colgroup>
            <col className="providers-table__id" />
            <col className="providers-table__address" />
            <col className="providers-table__protocol" />
            <col className="providers-table__auth" />
            <col className="providers-table__models" />
            <col className="providers-table__status" />
            <col className="providers-table__actions" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">Provider ID</th>
              <th scope="col">Base URL</th>
              <th scope="col">默认协议</th>
              <th scope="col">认证</th>
              <th scope="col">模型</th>
              <th scope="col">状态</th>
              <th scope="col">操作</th>
            </tr>
          </thead>
          <tbody>
            {providers.length === 0 ? (
              <tr className="providers-empty-row"><td colSpan={7}>{data.providers.length === 0 ? "尚未配置 Provider。" : "没有匹配的 Provider 或模型。"}</td></tr>
            ) : providers.map((provider) => <ProviderRows key={provider.id} provider={provider} />)}
          </tbody>
        </table>
      </div>
    </>
  );
}

function ProviderRows({ provider }: { provider: OverviewProvider }) {
  const status = providerStatus(provider);
  return (
    <Fragment>
      <tr className={provider.editable ? "providers-row" : "providers-row providers-row--readonly"}>
        <td>{provider.id}</td>
        <td>
          <div className="providers-address">
            {provider.name ? <span>{provider.name}</span> : null}
            <code>{provider.baseUrl ?? "未配置地址"}</code>
          </div>
        </td>
        <td>{provider.defaultApi ?? "由模型指定"}</td>
        <td>{providerAuthSummary(provider)}</td>
        <td>{provider.modelCount}</td>
        <td><StatusIndicator tone={status.tone}>{status.label}</StatusIndicator></td>
        <td className="providers-actions-cell">
          <Button
            type="button"
            variant="secondary"
            disabledAppearance="stable"
            aria-label={`${provider.id} 操作`}
            title={provider.readOnlyReason ?? "Provider 操作不可用。"}
            disabled
          >
            <MoreHorizontal aria-hidden="true" />
          </Button>
        </td>
      </tr>
      {!provider.editable && provider.readOnlyReason ? (
        <tr className="providers-readonly-reason"><td colSpan={7}><strong>只读原因</strong><span>{provider.readOnlyReason}</span></td></tr>
      ) : null}
    </Fragment>
  );
}
