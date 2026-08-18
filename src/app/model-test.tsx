import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "../components/ui";
import { asAppError, useTauriClient, type ModelTestResult, type OverviewModel, type OverviewProvider, type TauriClient } from "../lib/tauri-client";
import { useModelTestStore } from "../store/model-test";
import { useUiSettings } from "../store/ui-settings";
import { buildModelEndpoint } from "./model-endpoint";

export function isModelTestable(provider: OverviewProvider, model: OverviewModel, targetWritable: boolean): boolean {
  const endpoint = provider.baseUrl && model.effectiveApi
    ? buildModelEndpoint(provider.baseUrl, model.id, model.effectiveApi)
    : { kind: "not-configured" as const };
  return targetWritable
    && provider.editable
    && (provider.authMode !== "api-key" || provider.hasApiKey)
    && provider.authMode !== "unsupported"
    && model.editable
    && model.complete
    && model.contextWindow !== null
    && model.maxTokens !== null
    && model.maxTokens <= model.contextWindow
    && !model.unsupportedProtocol
    && !model.hasBaseUrlOverride
    && endpoint.kind === "available";
}

export function useRefreshAfterModelTest({
  ready,
  loading,
  revision,
  refresh,
}: {
  ready: boolean;
  loading: boolean;
  revision: number;
  refresh(): Promise<unknown>;
}) {
  const client = useTauriClient();
  const running = useModelTestStore((state) => state.running);
  const needsOverviewRefresh = useModelTestStore((state) => state.needsOverviewRefresh);
  const reconcileModelTest = useModelTestStore((state) => state.reconcile);
  const reconciledRevision = useRef<number | null>(null);

  useEffect(() => {
    if (!ready || loading || revision === 0 || reconciledRevision.current === revision) return;
    let active = true;
    let retryTimer: number | undefined;
    let failureNotified = false;
    const reconcile = async () => {
      const generation = useModelTestStore.getState().generation;
      try {
        const state = await client.getModelTestState();
        if (!active) return;
        reconcileModelTest(state, generation);
        reconciledRevision.current = revision;
        failureNotified = false;
      } catch (cause: unknown) {
        if (!active) return;
        if (!failureNotified) {
          const error = asAppError(cause, "无法同步模型测试状态");
          toast.error(error.message, { description: error.action });
          failureNotified = true;
        }
        retryTimer = window.setTimeout(() => {
          retryTimer = undefined;
          void reconcile();
        }, REMOTE_MODEL_TEST_STATE_POLL_MS);
      }
    };
    void reconcile();
    return () => {
      active = false;
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
    };
  }, [client, loading, ready, reconcileModelTest, revision]);

  useEffect(() => {
    if (!ready || loading || running || !needsOverviewRefresh) return;
    void (async () => {
      const refreshGeneration = useModelTestStore.getState().generation;
      const clearInvalidatedResult = () => {
        const current = useModelTestStore.getState();
        if (current.generation === refreshGeneration) current.prepareHydration();
      };
      try {
        const refreshError = await refresh();
        if (refreshError) {
          clearInvalidatedResult();
          const error = asAppError(refreshError, "无法刷新模型测试后的配置");
          toast.error(error.message, { description: error.action });
        }
      } catch (cause: unknown) {
        clearInvalidatedResult();
        const error = asAppError(cause, "无法刷新模型测试后的配置");
        toast.error(error.message, { description: error.action });
      }
    })();
  }, [loading, needsOverviewRefresh, ready, refresh, running]);
}

type PendingTest = { providerId: string; modelId: string };

const REMOTE_MODEL_TEST_STATE_POLL_MS = 250;

function syncRemoteModelTestState(client: Pick<TauriClient, "getModelTestState">, generation: number) {
  let failureNotified = false;
  const poll = async (): Promise<void> => {
    if (useModelTestStore.getState().generation !== generation) return;
    try {
      const state = await client.getModelTestState();
      const store = useModelTestStore.getState();
      if (store.generation !== generation) return;
      failureNotified = false;
      store.hydrate(state, generation);
      if (state.running && useModelTestStore.getState().generation === generation) {
        window.setTimeout(() => void poll(), REMOTE_MODEL_TEST_STATE_POLL_MS);
      }
    } catch (cause: unknown) {
      if (useModelTestStore.getState().generation !== generation) return;
      if (!failureNotified) {
        const error = asAppError(cause, "无法读取模型测试状态");
        toast.error(error.message, { description: error.action });
        failureNotified = true;
      }
      window.setTimeout(() => void poll(), REMOTE_MODEL_TEST_STATE_POLL_MS);
    }
  };
  void poll();
}

export function useModelTestRunner() {
  const client = useTauriClient();
  const hydrationState = useUiSettings((state) => state.hydrationState);
  const modelTestCostNoticeAccepted = useUiSettings((state) => state.modelTestCostNoticeAccepted);
  const running = useModelTestStore((state) => state.running);
  const activeProviderId = useModelTestStore((state) => state.providerId);
  const activeModelId = useModelTestStore((state) => state.modelId);
  const result = useModelTestStore((state) => state.result);
  const terminal = useModelTestStore((state) => state.terminal);
  const [pendingTest, setPendingTest] = useState<PendingTest | null>(null);

  const execute = useCallback(async (test: PendingTest) => {
    if (!useModelTestStore.getState().begin(test.providerId, test.modelId)) {
      toast.info("已有模型测试正在进行。");
      return;
    }
    try {
      const nextResult = await client.testModel(test);
      useModelTestStore.getState().finish(nextResult);
      notifyResult(nextResult);
    } catch (cause: unknown) {
      const error = asAppError(cause, "模型测试失败");
      if (error.code === "model-test-busy") {
        const generation = useModelTestStore.getState().recoverRemote();
        toast.info("已有模型测试正在进行。");
        syncRemoteModelTestState(client, generation);
        return;
      }
      if (error.code === "model-test-cancelled" || error.code === "model-test-timeout") {
        const generation = useModelTestStore.getState().recoverRemote();
        if (error.code === "model-test-timeout") {
          toast.error(error.message, { description: error.action });
        }
        syncRemoteModelTestState(client, generation);
        return;
      }
      useModelTestStore.getState().fail();
      toast.error(error.message, { description: error.action });
    }
  }, [client]);

  const start = useCallback((providerId: string, modelId: string) => {
    if (hydrationState !== "ready") return;
    const test = { providerId, modelId };
    if (useModelTestStore.getState().running) {
      toast.info("已有模型测试正在进行。");
      return;
    }
    if (!modelTestCostNoticeAccepted) {
      setPendingTest(test);
      return;
    }
    void execute(test);
  }, [execute, hydrationState, modelTestCostNoticeAccepted]);

  const cancel = useCallback(() => {
    void client.cancelModelTest().catch((cause: unknown) => {
      const error = asAppError(cause, "无法取消模型测试");
      toast.error(error.message, { description: error.action });
    });
  }, [client]);

  const confirmCostNotice = useCallback(async () => {
    if (!pendingTest) return;
    const test = pendingTest;
    try {
      const settings = await client.acceptModelTestCostNotice();
      useUiSettings.getState().setModelTestCostNoticeAccepted(settings.modelTestCostNoticeAccepted);
      setPendingTest(null);
      void execute(test);
    } catch (cause: unknown) {
      const error = asAppError(cause, "无法保存费用说明偏好");
      toast.error(error.message, { description: error.action });
    }
  }, [client, execute, pendingTest]);

  const costNoticeDialog = pendingTest ? (
    <ConfirmDialog
      title="模型测试费用说明"
      cancelLabel="取消"
      confirmLabel="继续测试"
      onCancel={() => setPendingTest(null)}
      onConfirm={() => void confirmCostNotice()}
    >
      模型测试会向 Provider 发起真实 API 请求，可能产生费用。
    </ConfirmDialog>
  ) : null;

  return {
    running,
    activeProviderId,
    activeModelId,
    terminal,
    result,
    start,
    settingsReady: hydrationState === "ready",
    cancel,
    isActive: (providerId: string, modelId: string) => running && activeProviderId === providerId && activeModelId === modelId,
    isBusy: (providerId: string, modelId: string) => running && !(activeProviderId === providerId && activeModelId === modelId),
    costNoticeDialog,
  };
}

function notifyResult(result: ModelTestResult) {
  if (result.errorCode === "cancelled") return;
  if (result.success) {
    toast.success(`模型连接成功 · ${result.latencyMs} ms`);
    return;
  }
  toast.error("模型测试失败", { description: modelTestFailureAction(result.errorCode) });
}

function modelTestFailureAction(errorCode: string | undefined): string {
  switch (errorCode) {
    case "http-401": return "检查 API Key 后重试。";
    case "http-403": return "检查 Provider 权限后重试。";
    case "http-404": return "检查 Model ID 和最终地址后重试。";
    case "http-429": return "检查额度或稍后重试。";
    case "timeout": return "检查网络和 Provider 响应后重试。";
    case "dns": return "检查 Provider 域名和网络后重试。";
    case "tls": return "检查 HTTPS 证书和地址后重试。";
    case "response-format": return "检查模型协议和 Provider 响应格式后重试。";
    default: return "检查 Provider 配置和网络后重试。";
  }
}

export type ModelTestRunner = ReturnType<typeof useModelTestRunner>;
