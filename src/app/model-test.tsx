import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "../components/ui";
import { asAppError, useTauriClient, type ModelTestResult, type OverviewModel, type OverviewProvider } from "../lib/tauri-client";
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
    && !model.unsupportedProtocol
    && !model.hasBaseUrlOverride
    && endpoint.kind === "available";
}

export function useRefreshAfterModelTest({
  ready,
  loading,
  providerId,
  refresh,
}: {
  ready: boolean;
  loading: boolean;
  providerId?: string;
  refresh(): Promise<unknown>;
}) {
  const result = useModelTestStore((state) => state.result);
  const running = useModelTestStore((state) => state.running);
  const refreshedResult = useRef<ModelTestResult | null>(null);

  useEffect(() => {
    if (running || !result || !ready || loading || (providerId && result.providerId !== providerId) || refreshedResult.current === result) return;
    refreshedResult.current = result;
    void refresh();
  }, [loading, providerId, ready, refresh, result, running]);
}

type PendingTest = { providerId: string; modelId: string };

export function useModelTestRunner() {
  const client = useTauriClient();
  const modelTestCostNoticeAccepted = useUiSettings((state) => state.modelTestCostNoticeAccepted);
  const running = useModelTestStore((state) => state.running);
  const activeProviderId = useModelTestStore((state) => state.providerId);
  const activeModelId = useModelTestStore((state) => state.modelId);
  const result = useModelTestStore((state) => state.result);
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
      useModelTestStore.getState().fail();
      const error = asAppError(cause, "模型测试失败");
      if (error.code !== "model-test-busy") toast.error(error.message, { description: error.action });
    }
  }, [client]);

  const start = useCallback((providerId: string, modelId: string) => {
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
  }, [execute, modelTestCostNoticeAccepted]);

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
    result,
    start,
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
  toast.error(result.message, { description: modelTestFailureAction(result.errorCode) });
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
