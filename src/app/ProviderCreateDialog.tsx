import { zodResolver } from "@hookform/resolvers/zod";
import { Eye, EyeOff, Info, LockKeyhole } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Controller, type Control, type FieldErrors, type UseFormRegister, useForm } from "react-hook-form";
import { z } from "zod";

import { Button, ConfirmDialog } from "../components/ui";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import {
  asAppError,
  type AppError,
  type CreateCustomProviderResult,
  type OverviewApi,
  useTauriClient,
} from "../lib/tauri-client";

const protocols = [
  "openai-completions",
  "openai-responses",
  "anthropic-messages",
  "google-generative-ai",
] as const satisfies readonly OverviewApi[];

const providerIdPattern = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const thinkingSuffixPattern = /:(?:off|minimal|low|medium|high|xhigh|max|auto)$/i;

const protocolSchema = z.enum(protocols);
const providerStepSchema = z.object({
  providerId: z
    .string()
    .trim()
    .min(1, "Provider ID 不能为空。")
    .refine(
      (value) => value.length === 0 || providerIdPattern.test(value),
      "Provider ID 只能包含字母、数字、.、_ 或 -。",
    ),
  baseUrl: z
    .string()
    .trim()
    .refine(isHttpUrl, "Base URL 必须是有效的 HTTP 或 HTTPS 地址。"),
  defaultApi: z.union([protocolSchema, z.literal("")]),
  authMode: z.enum(["api-key", "none"]),
  apiKey: z.string().refine((value) => !value.startsWith("!"), "Direct API Key 不能以 ! 开头。"),
});

const providerCreateSchema = providerStepSchema
  .extend({
    modelId: z
      .string()
      .trim()
      .min(1, "Model ID 不能为空。")
      .refine(
        (value) => value.length === 0 || (!/[\s\p{C}]/u.test(value) && !thinkingSuffixPattern.test(value)),
        "Model ID 不能包含空白、控制字符或 Thinking Level 后缀。",
      ),
    modelName: z.string().trim().min(1, "名称不能为空。"),
    modelApi: z.union([protocolSchema, z.literal("")]),
    inputText: z.boolean(),
    inputImage: z.boolean(),
    reasoning: z.boolean(),
    contextWindow: z.number().int("Context Window 必须是整数。").positive("Context Window 必须大于 0。"),
    maxTokens: z.number().int("Max Tokens 必须是整数。").positive("Max Tokens 必须大于 0。"),
  })
  .superRefine((value, context) => {
    if (!value.inputText && !value.inputImage) {
      context.addIssue({
        code: "custom",
        path: ["inputText"],
        message: "至少选择 Text 或 Image 一种能力。",
      });
    }
    if (!value.defaultApi && !value.modelApi) {
      context.addIssue({
        code: "custom",
        path: ["modelApi"],
        message: "请在默认协议或模型协议中选择一种受支持协议。",
      });
    }
    if (value.maxTokens > value.contextWindow) {
      context.addIssue({
        code: "custom",
        path: ["maxTokens"],
        message: "Max Tokens 不能超过 Context Window。",
      });
    }
  });

type ProviderCreateValues = z.infer<typeof providerCreateSchema>;
type ProviderStepValues = z.infer<typeof providerStepSchema>;
type ProviderCreateControl = Control<ProviderCreateValues>;
type ProviderCreateErrors = FieldErrors<ProviderCreateValues>;

type ModelField = "modelId" | "modelName" | "modelApi" | "inputText" | "inputImage" | "reasoning" | "contextWindow" | "maxTokens";
type ProviderCreateRegister = UseFormRegister<ProviderCreateValues>;

const providerStepFields = ["providerId", "baseUrl", "defaultApi", "authMode", "apiKey"] as const satisfies readonly (keyof ProviderStepValues)[];

type Step = "provider" | "model";

type ProviderCreateDialogProps = {
  openedModelsHash: string;
  onDismiss(): void;
  onReload(): Promise<void>;
  onCreated(result: CreateCustomProviderResult): Promise<void>;
};

export function ProviderCreateDialog({
  openedModelsHash,
  onDismiss,
  onReload,
  onCreated,
}: ProviderCreateDialogProps) {
  const client = useTauriClient();
  const [step, setStep] = useState<Step>("provider");
  const [modelSubmitAttempted, setModelSubmitAttempted] = useState(false);
  const [blurredModelFields, setBlurredModelFields] = useState<Partial<Record<ModelField, true>>>({});
  const [showApiKey, setShowApiKey] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<AppError | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const {
    clearErrors,
    control,
    formState: { errors, isDirty },
    getValues,
    handleSubmit,
    register,
    setError,
    watch,
  } = useForm<ProviderCreateValues>({
    resolver: zodResolver(providerCreateSchema),
    mode: "onBlur",
    reValidateMode: "onChange",
    defaultValues: {
      providerId: "",
      baseUrl: "",
      defaultApi: "",
      authMode: "api-key",
      apiKey: "",
      modelId: "",
      modelName: "",
      modelApi: "openai-responses",
      inputText: true,
      inputImage: true,
      reasoning: true,
      contextWindow: 356_000,
      maxTokens: 128_000,
    },
  });
  const values = watch();
  const apiKeyMode = values.authMode === "api-key";
  const finalAddress = endpointPreview(values);
  const markModelFieldBlurred = (field: ModelField) => {
    setBlurredModelFields((fields) => {
      if (fields[field]) return fields;
      return { ...fields, [field]: true };
    });
  };

  const requestDismiss = () => {
    if (submitting) return;
    if (isDirty) {
      setConfirmDiscard(true);
      return;
    }
    onDismiss();
  };

  const advance = () => {
    const validation = providerStepSchema.safeParse({
      providerId: getValues("providerId") ?? "",
      baseUrl: getValues("baseUrl") ?? "",
      defaultApi: getValues("defaultApi") ?? "",
      authMode: getValues("authMode") ?? "api-key",
      apiKey: getValues("apiKey") ?? "",
    });
    if (!validation.success) {
      clearErrors([...providerStepFields]);
      let focused = false;
      for (const issue of validation.error.issues) {
        const field = issue.path.at(0);
        if (typeof field !== "string" || !providerStepFields.includes(field as (typeof providerStepFields)[number])) continue;
        setError(field as (typeof providerStepFields)[number], {
          type: "validate",
          message: issue.message,
        }, { shouldFocus: !focused });
        focused = true;
      }
      return;
    }
    clearErrors([
      ...providerStepFields,
      "modelId",
      "modelName",
      "modelApi",
      "inputText",
      "inputImage",
      "reasoning",
      "contextWindow",
      "maxTokens",
    ]);
    setSubmissionError(null);
    setStep("model");
  };

  const submit = async (form: ProviderCreateValues) => {
    setSubmissionError(null);
    setSubmitting(true);
    try {
      const result = await client.createCustomProvider({
        openedModelsHash,
        provider: {
          id: form.providerId.trim(),
          baseUrl: form.baseUrl.trim().replace(/\/+$/, ""),
          defaultApi: form.defaultApi || undefined,
          authMode: form.authMode,
          apiKey: form.authMode === "api-key" && form.apiKey ? form.apiKey : undefined,
        },
        firstModel: {
          id: form.modelId.trim(),
          name: form.modelName,
          api: form.modelApi || undefined,
          reasoning: form.reasoning,
          input: [form.inputText && "text", form.inputImage && "image"].filter(
            (input): input is "text" | "image" => Boolean(input),
          ),
          contextWindow: form.contextWindow,
          maxTokens: form.maxTokens,
        },
      });
      await onCreated(result);
    } catch (cause: unknown) {
      const error = asAppError(cause, "创建 Provider 失败");
      const field = errorField(error.code);
      if (field) {
        setError(field, { type: "server", message: error.message }, { shouldFocus: true });
      } else {
        setSubmissionError(error);
      }
    } finally {
      setSubmitting(false);
    }
  };

  const submitForm = () => {
    setModelSubmitAttempted(true);
    void handleSubmit(submit)();
  };

  const reloadAfterConflict = async () => {
    await onReload();
    onDismiss();
  };

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open) requestDismiss(); }}>
        <DialogContent
          aria-describedby="provider-create-description"
          className={`provider-create-dialog ${step === "model" ? "provider-create-dialog--model" : ""}`}
          onEscapeKeyDown={(event) => {
            event.preventDefault();
            requestDismiss();
          }}
          onPointerDownOutside={(event) => {
            event.preventDefault();
            requestDismiss();
          }}
        >
          <form className="provider-create-form" noValidate onSubmit={(event) => event.preventDefault()}>
            <div className="provider-create-form__body">
              <header className="provider-create-heading">
                <DialogTitle>新增 Provider</DialogTitle>
                <p className="provider-create-step">{step === "provider" ? "步骤 1 / 2 · Provider" : "步骤 2 / 2 · 首个模型"}</p>
                <DialogDescription id="provider-create-description">
                  {step === "provider"
                    ? "先配置连接和认证，下一步添加首个模型。"
                    : "Provider 只有和首个模型一起验证并保存后才会创建。"}
                </DialogDescription>
              </header>

              {step === "provider" ? (
                <ProviderStep
                  control={control}
                  errors={errors}
                  register={register}
                  showApiKey={showApiKey}
                  apiKeyMode={apiKeyMode}
                  onToggleApiKey={() => setShowApiKey((visible) => !visible)}
                />
              ) : (
                <ModelStep
                  control={control}
                  errors={errors}
                  register={register}
                  values={values}
                  finalAddress={finalAddress}
                  shouldShowError={(field) => modelSubmitAttempted || Boolean(blurredModelFields[field])}
                  onFieldBlur={markModelFieldBlurred}
                />
              )}

              {submissionError ? (
                <section className="provider-create-submit-error" role="alert" aria-live="assertive">
                  <div>
                    <strong>{submissionError.code === "models-hash-conflict" ? "配置冲突" : "无法创建 Provider"}</strong>
                    <p>{submissionError.message}</p>
                    <p>{submissionError.action}</p>
                  </div>
                  {submissionError.code === "models-hash-conflict" ? (
                    <Button type="button" variant="secondary" onClick={() => void reloadAfterConflict()}>
                      重新读取
                    </Button>
                  ) : null}
                </section>
              ) : null}
            </div>

            <footer className="provider-create-footer">
              {step === "model" ? (
                <Button type="button" variant="secondary" disabled={submitting} onClick={() => setStep("provider")}>
                  返回
                </Button>
              ) : <span />}
              <div className="provider-create-footer__actions">
                <Button type="button" variant="secondary" disabled={submitting} onClick={requestDismiss}>
                  取消
                </Button>
                {step === "provider" ? (
                  <Button type="button" onClick={() => void advance()}>
                    下一步
                  </Button>
                ) : (
                  <Button type="button" disabled={submitting} aria-busy={submitting} onClick={submitForm}>
                    {submitting ? "创建中…" : "创建 Provider"}
                  </Button>
                )}
              </div>
            </footer>
          </form>
        </DialogContent>
      </Dialog>

      {confirmDiscard ? (
        <ConfirmDialog
          title="有未保存的修改"
          onCancel={() => setConfirmDiscard(false)}
          onConfirm={onDismiss}
        >
          离开后，这些修改将会丢失。
        </ConfirmDialog>
      ) : null}
    </>
  );
}

function ProviderStep({
  control,
  errors,
  register,
  showApiKey,
  apiKeyMode,
  onToggleApiKey,
}: {
  control: ProviderCreateControl;
  errors: ProviderCreateErrors;
  register: ProviderCreateRegister;
  showApiKey: boolean;
  apiKeyMode: boolean;
  onToggleApiKey(): void;
}) {
  return (
    <section className="provider-create-fields" aria-label="Provider 配置">
      <FormRow label="Provider ID" htmlFor="provider-id" error={errors.providerId?.message}>
        <Input id="provider-id" autoComplete="off" aria-invalid={Boolean(errors.providerId)} {...register("providerId")} />
      </FormRow>
      <FormRow label="Base URL" htmlFor="provider-base-url" error={errors.baseUrl?.message}>
        <Input id="provider-base-url" type="url" inputMode="url" autoComplete="url" aria-invalid={Boolean(errors.baseUrl)} {...register("baseUrl")} />
      </FormRow>
      <FormRow label="默认协议（可选）" htmlFor="provider-default-api" error={errors.defaultApi?.message}>
        <ProtocolSelect control={control} name="defaultApi" id="provider-default-api" inheritLabel="由模型指定" />
      </FormRow>
      <fieldset className="provider-create-auth">
        <legend>认证方式</legend>
        <div className="provider-create-auth__choices" role="radiogroup" aria-label="认证方式">
          <label className="provider-create-auth__choice">
            <input type="radio" value="api-key" aria-label="API Key 认证" {...register("authMode")} />
            <span>API Key</span>
          </label>
          <label className="provider-create-auth__choice">
            <input type="radio" value="none" aria-label="无需认证" {...register("authMode")} />
            <span>无需认证</span>
          </label>
        </div>
      </fieldset>
      {apiKeyMode ? (
        <FormRow label="API Key" htmlFor="provider-api-key" error={errors.apiKey?.message}>
          <div className="provider-create-key-input">
            <Input
              id="provider-api-key"
              type={showApiKey ? "text" : "password"}
              autoComplete="new-password"
              aria-invalid={Boolean(errors.apiKey)}
              {...register("apiKey")}
            />
            <button type="button" aria-label={showApiKey ? "隐藏 API Key" : "显示 API Key"} onClick={onToggleApiKey}>
              {showApiKey ? <EyeOff aria-hidden="true" /> : <Eye aria-hidden="true" />}
            </button>
          </div>
        </FormRow>
      ) : null}
      <p className="provider-create-security-note"><LockKeyhole aria-hidden="true" />密钥直接写入 OMP 配置，不会保存在应用中。</p>
    </section>
  );
}

function ModelStep({
  control,
  errors,
  register,
  values,
  finalAddress,
  shouldShowError,
  onFieldBlur,
}: {
  control: ProviderCreateControl;
  errors: ProviderCreateErrors;
  register: ProviderCreateRegister;
  values: ProviderCreateValues;
  finalAddress: string;
  shouldShowError(field: ModelField): boolean;
  onFieldBlur(field: ModelField): void;
}) {
  const defaultApi = values.defaultApi || "由模型指定";
  const authentication = values.authMode === "api-key" && values.apiKey ? "API Key 已填写" : values.authMode === "api-key" ? "API Key 未填写" : "无需认证";
  const modelIdError = shouldShowError("modelId") ? errors.modelId?.message : undefined;
  const modelNameError = shouldShowError("modelName") ? errors.modelName?.message : undefined;
  const modelApiError = shouldShowError("modelApi") ? errors.modelApi?.message : undefined;
  const capabilityError = shouldShowError("inputText") || shouldShowError("inputImage")
    ? errors.inputText?.message ?? errors.inputImage?.message
    : undefined;
  const contextWindowError = shouldShowError("contextWindow") ? errors.contextWindow?.message : undefined;
  const maxTokensError = shouldShowError("maxTokens") ? errors.maxTokens?.message : undefined;
  const modelId = register("modelId");
  const modelName = register("modelName");
  const inputText = register("inputText");
  const inputImage = register("inputImage");
  const reasoning = register("reasoning");
  const contextWindow = register("contextWindow", { valueAsNumber: true });
  const maxTokens = register("maxTokens", { valueAsNumber: true });

  return (
    <section className="provider-create-model" aria-label="首个 Model definition">
      <div className="provider-create-summary" aria-label="Provider 摘要">
        <code>{values.providerId.trim() || "Provider ID"}</code>
        <span>·</span>
        <code>{values.baseUrl.trim() || "Base URL"}</code>
        <span>·</span>
        <span>默认协议</span>
        <strong>{defaultApi}</strong>
        <span>·</span>
        <span>{authentication}</span>
      </div>
      <div className="provider-create-fields">
        <FormRow label="Model ID" htmlFor="provider-model-id" error={modelIdError}>
          <Input
            id="provider-model-id"
            autoComplete="off"
            aria-invalid={Boolean(modelIdError)}
            {...modelId}
            onBlur={(event) => {
              modelId.onBlur(event);
              onFieldBlur("modelId");
            }}
          />
        </FormRow>
        <FormRow label="名称" htmlFor="provider-model-name" error={modelNameError}>
          <Input
            id="provider-model-name"
            autoComplete="off"
            aria-invalid={Boolean(modelNameError)}
            {...modelName}
            onBlur={(event) => {
              modelName.onBlur(event);
              onFieldBlur("modelName");
            }}
          />
        </FormRow>
        <FormRow label="协议" htmlFor="provider-model-api" error={modelApiError}>
          <ProtocolSelect
            control={control}
            name="modelApi"
            id="provider-model-api"
            inheritLabel={values.defaultApi ? `继承 ${values.defaultApi}` : "请选择协议"}
          />
        </FormRow>
        <fieldset className="provider-create-capabilities">
          <legend>能力</legend>
          <div>
            <label><input type="checkbox" {...inputText} onBlur={(event) => { inputText.onBlur(event); onFieldBlur("inputText"); }} />Text</label>
            <label><input type="checkbox" {...inputImage} onBlur={(event) => { inputImage.onBlur(event); onFieldBlur("inputImage"); }} />Image</label>
            <label><input type="checkbox" {...reasoning} onBlur={(event) => { reasoning.onBlur(event); onFieldBlur("reasoning"); }} />Reasoning</label>
          </div>
          <p className="provider-create-field-error" aria-live="polite">{capabilityError ?? ""}</p>
        </fieldset>
        <FormRow label="Context Window" htmlFor="provider-context-window" error={contextWindowError}>
          <Input
            id="provider-context-window"
            type="number"
            min="1"
            inputMode="numeric"
            aria-invalid={Boolean(contextWindowError)}
            {...contextWindow}
            onBlur={(event) => {
              contextWindow.onBlur(event);
              onFieldBlur("contextWindow");
            }}
          />
        </FormRow>
        <FormRow label="Max Tokens" htmlFor="provider-max-tokens" error={maxTokensError}>
          <Input
            id="provider-max-tokens"
            type="number"
            min="1"
            inputMode="numeric"
            aria-invalid={Boolean(maxTokensError)}
            {...maxTokens}
            onBlur={(event) => {
              maxTokens.onBlur(event);
              onFieldBlur("maxTokens");
            }}
          />
        </FormRow>
        <FormRow label="最终地址" htmlFor="provider-final-address" error={undefined}>
          <Input id="provider-final-address" readOnly value={finalAddress} />
        </FormRow>
      </div>
      <p className="provider-create-model-note"><Info aria-hidden="true" />将一次写入 Provider 和首个模型；失败时不会留下空 Provider。</p>
    </section>
  );
}

function FormRow({
  label,
  htmlFor,
  error,
  children,
}: {
  label: string;
  htmlFor: string;
  error?: string;
  children: ReactNode;
}) {
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

function ProtocolSelect({
  control,
  name,
  id,
  inheritLabel,
}: {
  control: ProviderCreateControl;
  name: "defaultApi" | "modelApi";
  id: string;
  inheritLabel: string;
}) {
  return (
    <Controller
      control={control}
      name={name}
      render={({ field }) => (
        <Select value={field.value || "inherit"} onValueChange={(value) => field.onChange(value === "inherit" ? "" : value)}>
          <SelectTrigger id={id} aria-label={name === "defaultApi" ? "默认协议（可选）" : "协议"} className="provider-create-select">
            <SelectValue placeholder={inheritLabel} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="inherit">{inheritLabel}</SelectItem>
            {protocols.map((protocol) => <SelectItem key={protocol} value={protocol}>{protocol}</SelectItem>)}
          </SelectContent>
        </Select>
      )}
    />
  );
}

function errorField(code: string): "providerId" | "baseUrl" | "apiKey" | "modelId" | "modelName" | "modelApi" | "inputText" | "contextWindow" | "maxTokens" | null {
  switch (code) {
    case "provider-id-invalid":
    case "provider-id-conflict":
      return "providerId";
    case "provider-base-url-invalid":
      return "baseUrl";
    case "provider-api-key-invalid":
    case "provider-auth-invalid":
      return "apiKey";
    case "model-id-invalid":
    case "model-id-conflict":
      return "modelId";
    case "model-name-required":
      return "modelName";
    case "model-api-required":
      return "modelApi";
    case "model-input-required":
      return "inputText";
    case "model-context-window-invalid":
      return "contextWindow";
    case "model-token-limit-invalid":
      return "maxTokens";
    default:
      return null;
  }
}

function endpointPreview(values: ProviderCreateValues): string {
  const baseUrl = values.baseUrl.trim().replace(/\/+$/, "");
  const modelId = values.modelId.trim();
  const protocol = values.modelApi || values.defaultApi;
  if (!isHttpUrl(baseUrl) || !modelId || !protocol) {
    return "填写有效 Provider、Model 和协议后显示最终地址";
  }
  switch (protocol) {
    case "openai-completions":
      return `${baseUrl}/chat/completions`;
    case "openai-responses":
      return `${baseUrl}/responses`;
    case "anthropic-messages":
      return `${baseUrl}/v1/messages`;
    case "google-generative-ai":
      return `${baseUrl}/models/${modelId}:streamGenerateContent?alt=sse`;
  }
}

function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && Boolean(url.hostname);
  } catch {
    return false;
  }
}
