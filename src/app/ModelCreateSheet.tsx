import { zodResolver } from "@hookform/resolvers/zod";
import { Info, LockKeyhole } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Controller, useForm } from "react-hook-form";
import { type BlockerFunction, useBlocker } from "react-router";
import { z } from "zod";

import { Button, ConfirmDialog } from "../components/ui";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import {
  asAppError,
  type AppError,
  type EditModelInput,
  type ModelMutationResult,
  type OverviewApi,
  type OverviewModel,
  type OverviewProvider,
  useTauriClient,
} from "../lib/tauri-client";
import { buildModelEndpoint, isHttpUrl } from "./model-endpoint";

const protocols = [
  "openai-completions",
  "openai-responses",
  "anthropic-messages",
  "google-generative-ai",
] as const satisfies readonly OverviewApi[];

const modelSchema = z.object({
  modelId: z.string().trim().min(1, "Model ID 不能为空。"),
  name: z.string().trim().min(1, "名称不能为空。"),
  modelApi: z.union([z.enum(protocols), z.literal("")]),
  inputText: z.boolean(),
  inputImage: z.boolean(),
  reasoning: z.boolean(),
  contextWindow: z.union([z.number().int("Context Window 必须是整数。").positive("Context Window 必须大于 0。"), z.literal("")]),
  maxTokens: z.union([z.number().int("Max Tokens 必须是整数。").positive("Max Tokens 必须大于 0。"), z.literal("")]),
}).superRefine((value, context) => {
  if (!value.inputText && !value.inputImage) {
    context.addIssue({ code: "custom", path: ["inputText"], message: "至少选择 Text 或 Image 一种能力。" });
  }
  if (typeof value.maxTokens === "number" && typeof value.contextWindow === "number" && value.maxTokens > value.contextWindow) {
    context.addIssue({ code: "custom", path: ["maxTokens"], message: "Max Tokens 不能大于 Context Window。" });
  }
});

type ModelFormValues = z.infer<typeof modelSchema>;

type ModelCreateSheetProps = {
  provider: OverviewProvider;
  openedModelsHash: string;
  mode: "create" | "edit";
  model?: OverviewModel;
  copy?: boolean;
  onDismiss(): void;
  onReload(): Promise<AppError | null>;
  onSaved(result: ModelMutationResult): Promise<AppError | null>;
};

export function ModelCreateSheet({
  provider,
  openedModelsHash,
  mode,
  model,
  copy = false,
  onDismiss,
  onReload,
  onSaved,
}: ModelCreateSheetProps) {
  const client = useTauriClient();
  const [submitting, setSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<AppError | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const submissionInFlight = useRef(false);
  const successfulSubmission = useRef(false);
  const feedbackRef = useRef<HTMLElement>(null);
  const source = model;
  const isEditing = mode === "edit" && Boolean(source);
  const defaultModelId = isEditing ? source?.id ?? "" : copy && source ? `${source.id}-copy` : "";
  const repairMode = isEditing && !copy;
  const defaultValues: ModelFormValues = {
    modelId: defaultModelId,
    name: source?.name ?? "",
    modelApi: source?.apiSource === "model" ? source.effectiveApi ?? "" : "",
    inputText: source ? source.input.includes("text") : true,
    inputImage: source ? source.input.includes("image") : true,
    reasoning: source ? source.reasoning ?? false : true,
    contextWindow: source?.contextWindow ?? (repairMode ? "" : 356_000),
    maxTokens: source?.maxTokens ?? (repairMode ? "" : 128_000),
  };
  const {
    control,
    formState: { errors, isDirty },
    handleSubmit,
    register,
    setError,
    watch,
  } = useForm<ModelFormValues>({
    resolver: zodResolver(modelSchema),
    mode: "onBlur",
    reValidateMode: "onChange",
    defaultValues,
  });
  const values = watch();
  const protocol = values.modelApi || provider.defaultApi;
  const endpoint = isHttpUrl(provider.baseUrl ?? "") && protocol
    ? buildModelEndpoint(provider.baseUrl ?? "", values.modelId.trim(), protocol)
    : null;
  const endpointText = endpoint?.kind === "available" ? endpoint.value : "填写有效 Model ID 和协议后显示最终地址";
  const canSave = isDirty && modelSchema.safeParse(values).success && !submitting;

  useEffect(() => {
    if (submissionError) feedbackRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [submissionError]);

  const blocker = useBlocker(useCallback<BlockerFunction>(({ currentLocation, nextLocation }) => {
    const currentPath = `${currentLocation.pathname}${currentLocation.search}${currentLocation.hash}`;
    const nextPath = `${nextLocation.pathname}${nextLocation.search}${nextLocation.hash}`;
    return !successfulSubmission.current && isDirty && !submitting && currentPath !== nextPath;
  }, [isDirty, submitting]));

  useEffect(() => {
    if (blocker.state === "blocked") setConfirmDiscard(true);
  }, [blocker.state]);

  const requestDismiss = useCallback(() => {
    if (submitting) return;
    if (isDirty) {
      setConfirmDiscard(true);
      return;
    }
    onDismiss();
  }, [isDirty, onDismiss, submitting]);

  const submit = async (form: ModelFormValues) => {
    if (submissionInFlight.current) return;
    submissionInFlight.current = true;
    successfulSubmission.current = false;
    setSubmitting(true);
    setSubmissionError(null);
    try {
      if (typeof form.contextWindow !== "number" || typeof form.maxTokens !== "number") return;
      const input = {
        name: form.name.trim(),
        api: form.modelApi || undefined,
        reasoning: form.reasoning,
        input: [form.inputText && "text", form.inputImage && "image"].filter(
          (value): value is "text" | "image" => Boolean(value),
        ),
        contextWindow: form.contextWindow,
        maxTokens: form.maxTokens,
      };
      const result = isEditing && source
        ? await client.editModel({
            openedModelsHash,
            providerId: provider.id,
            modelId: source.id,
            model: input,
          } satisfies EditModelInput)
        : await client.createModel({
            openedModelsHash,
            providerId: provider.id,
            model: { id: form.modelId.trim(), ...input },
          });
      successfulSubmission.current = true;
      const reloadError = await onSaved(result);
      if (reloadError) {
        successfulSubmission.current = false;
        setSubmissionError(reloadError);
      }
    } catch (cause: unknown) {
      const error = asAppError(cause, isEditing ? "保存 Model 失败" : "创建 Model 失败");
      const field = errorField(error.code);
      if (field) {
        setError(field, { type: "server", message: error.message }, { shouldFocus: true });
      } else {
        setSubmissionError(error);
      }
    } finally {
      submissionInFlight.current = false;
      setSubmitting(false);
    }
  };

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open) requestDismiss(); }}>
        <DialogContent
          aria-describedby="model-create-sheet-description"
          className="provider-create-dialog provider-create-dialog--model model-create-sheet"
          onEscapeKeyDown={(event) => {
            event.preventDefault();
            requestDismiss();
          }}
          onPointerDownOutside={(event) => {
            event.preventDefault();
            requestDismiss();
          }}
          onKeyDown={(event) => {
            if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "s") return;
            event.preventDefault();
            void handleSubmit(submit)();
          }}
        >
          <form className="provider-create-form" noValidate onSubmit={(event) => { event.preventDefault(); void handleSubmit(submit)(); }}>
            <div className="provider-create-form__body">
              <header className="provider-create-heading">
                <DialogTitle>{isEditing ? "编辑模型" : "新增模型"}</DialogTitle>
                <DialogDescription id="model-create-sheet-description">
                  {isEditing ? `编辑 ${provider.id}/${source?.id ?? "Model definition"}。保存后才能进行连接测试。` : `添加到 ${provider.id}。保存后才能进行连接测试。`}
                </DialogDescription>
              </header>
              <div className="provider-create-fields">
                <FormRow label="Model ID" htmlFor="model-sheet-id" error={errors.modelId?.message}>
                  <Input
                    id="model-sheet-id"
                    readOnly={isEditing}
                    aria-invalid={Boolean(errors.modelId)}
                    {...register("modelId")}
                  />
                  {isEditing ? <p className="provider-edit-field-note"><LockKeyhole aria-hidden="true" />Stable ID 创建后不可修改</p> : null}
                </FormRow>
                <FormRow label="名称" htmlFor="model-sheet-name" error={errors.name?.message}>
                  <Input id="model-sheet-name" autoComplete="off" aria-invalid={Boolean(errors.name)} {...register("name")} />
                </FormRow>
                <FormRow label="协议" htmlFor="model-sheet-api" error={errors.modelApi?.message}>
                  <Controller
                    control={control}
                    name="modelApi"
                    render={({ field }) => (
                      <Select value={field.value || "inherit"} onValueChange={(value) => field.onChange(value === "inherit" ? "" : value)}>
                        <SelectTrigger id="model-sheet-api" aria-label="协议" className="provider-create-select">
                          <SelectValue placeholder={provider.defaultApi ? `继承 ${provider.defaultApi}` : "请选择协议"} />
                          {field.value ? <span className="provider-create-select__source" aria-hidden="true">模型指定</span> : null}
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="inherit">{provider.defaultApi ? `继承 ${provider.defaultApi}` : "继承 Provider（未设置）"}</SelectItem>
                          {protocols.map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}
                        </SelectContent>
                      </Select>
                    )}
                  />
                </FormRow>
                <fieldset className="provider-create-capabilities">
                  <legend>能力</legend>
                  <div>
                    <label><input type="checkbox" {...register("inputText")} />Text</label>
                    <label><input type="checkbox" {...register("inputImage")} />Image</label>
                    <label><input type="checkbox" {...register("reasoning")} />Reasoning</label>
                  </div>
                  <p className="provider-create-field-error" aria-live="polite">{errors.inputText?.message ?? errors.inputImage?.message ?? ""}</p>
                </fieldset>
                <FormRow label="Context Window" htmlFor="model-sheet-context" error={errors.contextWindow?.message}>
                  <Input id="model-sheet-context" type="number" min="1" inputMode="numeric" aria-invalid={Boolean(errors.contextWindow)} {...register("contextWindow", { valueAsNumber: true })} />
                </FormRow>
                <FormRow label="Max Tokens" htmlFor="model-sheet-max" error={errors.maxTokens?.message}>
                  <Input id="model-sheet-max" type="number" min="1" inputMode="numeric" aria-invalid={Boolean(errors.maxTokens)} {...register("maxTokens", { valueAsNumber: true })} />
                </FormRow>
                <FormRow label="最终地址" htmlFor="model-sheet-endpoint">
                  <Input id="model-sheet-endpoint" readOnly value={endpointText} />
                </FormRow>
              </div>
              <p className="provider-create-model-note"><Info aria-hidden="true" />模型只修改所属 Provider 下的目标路径；未知配置会原样保留。</p>
              {submissionError ? (
                <section ref={feedbackRef} className="provider-create-submit-error" role="alert" aria-live="assertive">
                  <div><strong>{submissionError.code === "models-hash-conflict" ? "配置冲突" : "无法保存 Model"}</strong><p>{submissionError.message}</p><p>{submissionError.action}</p></div>
                  {submissionError.code === "models-hash-conflict" ? <Button type="button" variant="secondary" onClick={() => void onReload()}>重新读取</Button> : null}
                </section>
              ) : null}
            </div>
            <footer className="provider-create-footer">
              <div className="model-create-sheet-test"><Button type="button" variant="secondary" disabled title="请先保存 Model definition">测试模型</Button><span>仅可测试已保存模型</span></div>
              <div className="provider-create-footer__actions">
                <Button type="button" variant="secondary" disabled={submitting} onClick={requestDismiss}>取消</Button>
                <Button type="submit" disabled={!canSave} aria-busy={submitting}>{submitting ? "保存中…" : "保存模型"}</Button>
              </div>
            </footer>
          </form>
        </DialogContent>
      </Dialog>
      {confirmDiscard ? (
        <ConfirmDialog
          title="有未保存的修改"
          cancelLabel="继续编辑"
          confirmLabel="放弃修改"
          onCancel={() => {
            if (blocker.state === "blocked") blocker.reset();
            setConfirmDiscard(false);
          }}
          onConfirm={() => {
            if (blocker.state === "blocked") {
              blocker.proceed();
              return;
            }
            onDismiss();
          }}
        >
          离开后，这些修改将会丢失。
        </ConfirmDialog>
      ) : null}
    </>
  );
}

function FormRow({ label, htmlFor, error, children }: { label: string; htmlFor: string; error?: string; children: React.ReactNode }) {
  return (
    <div className="provider-create-field">
      <label htmlFor={htmlFor}>{label}</label>
      <div>
        {children}
        <p id={`${htmlFor}-error`} className="provider-create-field-error" aria-live="polite">{error ?? ""}</p>
      </div>
    </div>
  );
}

function errorField(code: string): keyof ModelFormValues | null {
  switch (code) {
    case "model-id-invalid":
    case "model-id-conflict":
    case "model-id-immutable":
      return "modelId";
    case "model-name-required":
      return "name";
    case "model-api-required":
      return "modelApi";
    case "model-input-required":
    case "model-input-invalid":
      return "inputText";
    case "model-context-window-invalid":
      return "contextWindow";
    case "model-token-limit-invalid":
      return "maxTokens";
    default:
      return null;
  }
}
