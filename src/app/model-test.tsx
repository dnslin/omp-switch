import { useCallback, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "../components/ui";
import { asAppError, useTauriClient, type ModelTestResult, type OverviewModel, type OverviewProvider } from "../lib/tauri-client";
import { useModelTestStore } from "../store/model-test";
import { useUiSettings } from "../store/ui-settings";
import { buildModelEndpoint } from "./model-endpoint";

export function isModelTestable(provider: OverviewProvider, model: OverviewModel): boolean {
  const endpoint = provider.baseUrl && model.effectiveApi
    ? buildModelEndpoint(provider.baseUrl, model.id, model.effectiveApi)
    : { kind: "not-configured" as const };
  return provider.editable
    && (provider.authMode !== "api-key" || provider.hasApiKey)
    && provider.authMode !== "unsupported"
    && model.editable
    && model.complete
    && !model.unsupportedProtocol
    && !model.hasBaseUrlOverride
    && endpoint.kind === "available";
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
  toast.error("模型测试失败");
}

export type ModelTestRunner = ReturnType<typeof useModelTestRunner>;
