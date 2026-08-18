import { zodResolver } from "@hookform/resolvers/zod";
import { Eye, EyeOff, LockKeyhole } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Controller, useForm } from "react-hook-form";
import { type BlockerFunction, useBlocker } from "react-router";
import { toast } from "sonner";
import { z } from "zod";

import { Button, ConfirmDialog } from "../components/ui";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import {
  asAppError,
  type AppError,
  type EditCustomProviderInput,
  type EditCustomProviderResult,
  type OverviewApi,
  type OverviewProvider,
  useTauriClient,
} from "../lib/tauri-client";
import { isHttpUrl } from "./model-endpoint";

const protocols = [
  "openai-completions",
  "openai-responses",
  "anthropic-messages",
  "google-generative-ai",
] as const satisfies readonly OverviewApi[];

const protocolSchema = z.enum(protocols);
const directApiKeySchema = z.string().superRefine((value, context) => {
  if (!value) return;
  const masked = value.trim();
  if (masked.startsWith("!")) {
    context.addIssue({
      code: "custom",
      message: "Direct API Key 不能以 ! 开头。",
    });
  } else if (
    masked
    && [...masked].every((character) => "*•●▪█xX".includes(character)
  )) {
    context.addIssue({
      code: "custom",
      message: "掩码文本不能作为新的 Direct API Key。",
    });
  }
});

const providerEditSchema = z.object({
  baseUrl: z
    .string()
    .trim()
    .refine(isHttpUrl, "Base URL 必须是有效的 HTTP 或 HTTPS 地址。"),
  defaultApi: z.union([protocolSchema, z.literal("")]),
  authMode: z.enum(["api-key", "none"]),
  apiKey: directApiKeySchema,
});


type ProviderEditValues = z.infer<typeof providerEditSchema>;

type ProviderEditDialogProps = {
  provider: OverviewProvider;
  openedModelsHash: string;
  onDismiss(): void;
  onReload(): Promise<AppError | null>;
  onSaved(result: EditCustomProviderResult): Promise<AppError | null>;
};

export function ProviderEditDialog({
  provider,
  openedModelsHash,
  onDismiss,
  onReload,
  onSaved,
}: ProviderEditDialogProps) {
  const client = useTauriClient();
  const [showApiKey, setShowApiKey] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<AppError | null>(null);
  const [postSaveError, setPostSaveError] = useState<AppError | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmNoAuth, setConfirmNoAuth] = useState(false);
  const [confirmConflictReload, setConfirmConflictReload] = useState(false);
  const submissionInFlight = useRef(false);
  const successfulSubmission = useRef(false);
  const feedbackRef = useRef<HTMLElement>(null);
  const startedWithApiKey = provider.authMode === "api-key";
  const defaultValues: ProviderEditValues = {
    baseUrl: provider.baseUrl ?? "",
    defaultApi: provider.defaultApi ?? "",
    authMode: startedWithApiKey ? "api-key" : "none",
    apiKey: "",
  };
  const validationSchema = providerEditSchema;
  const {
    clearErrors,
    control,
    formState: { errors, isDirty },
    handleSubmit,
    register,
    reset,
    setError,
    setValue,
    watch,
  } = useForm<ProviderEditValues>({
    resolver: zodResolver(validationSchema),
    mode: "onBlur",
    reValidateMode: "onChange",
    defaultValues,
  });
  const values = watch();
  const requiresReplacement = values.authMode === "api-key" && !startedWithApiKey;
  const canSave = isDirty
    && validationSchema.safeParse(values).success
    && (!requiresReplacement || values.apiKey.trim().length > 0);

  useEffect(() => {
    if (!submissionError && !postSaveError) return;
    feedbackRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [postSaveError, submissionError]);

  const blocker = useBlocker(useCallback<BlockerFunction>(({ currentLocation, nextLocation }) => {
    const currentPath = `${currentLocation.pathname}${currentLocation.search}${currentLocation.hash}`;
    const nextPath = `${nextLocation.pathname}${nextLocation.search}${nextLocation.hash}`;
    return !successfulSubmission.current && isDirty && !submitting && currentPath !== nextPath;
  }, [isDirty, submitting]));

  useEffect(() => {
    if (blocker.state === "blocked") setConfirmDiscard(true);
  }, [blocker.state]);

  const dismissAfterClearingSecret = useCallback(() => {
    reset({ ...defaultValues, apiKey: "" });
    onDismiss();
  }, [defaultValues, onDismiss, reset]);

  const requestDismiss = useCallback(() => {
    if (submitting) return;
    if (isDirty) {
      setConfirmDiscard(true);
      return;
    }
    dismissAfterClearingSecret();
  }, [dismissAfterClearingSecret, isDirty, submitting]);

  const applyNoAuthentication = useCallback(() => {
    setValue("authMode", "none", { shouldDirty: true, shouldValidate: true });
    setValue("apiKey", "", { shouldDirty: true, shouldValidate: true });
    clearErrors("apiKey");
  }, [clearErrors, setValue]);

  const requestNoAuthentication = useCallback(() => {
    if (values.authMode === "none") return;
    if (provider.hasApiKey) {
      setConfirmNoAuth(true);
      return;
    }
    applyNoAuthentication();
  }, [applyNoAuthentication, provider.hasApiKey, values.authMode]);

  const submit = async (form: ProviderEditValues) => {
    if (submissionInFlight.current || postSaveError) return;
    if (form.authMode === "api-key" && !startedWithApiKey && !form.apiKey.trim()) {
      setError("apiKey", {
        type: "validate",
        message: "为 API Key 认证输入新的 Direct API Key。",
      }, { shouldFocus: true });
      return;
    }
    submissionInFlight.current = true;
    successfulSubmission.current = false;
    setSubmitting(true);
    setSubmissionError(null);
    try {
      const result = await client.editCustomProvider({
        openedModelsHash,
        providerId: provider.id,
        baseUrl: form.baseUrl.trim().replace(/\/+$/, ""),
        defaultApi: form.defaultApi || undefined,
        authMode: form.authMode,
        apiKey: keyIntent(form, startedWithApiKey),
      });
      reset({ ...form, apiKey: "" });
      successfulSubmission.current = true;
      const reloadError = await onSaved(result);
      if (reloadError) {
        setPostSaveError(reloadError);
        return;
      }
      toast.success("Provider 已保存");
      dismissAfterClearingSecret();
    } catch (cause: unknown) {
      successfulSubmission.current = false;
      const error = asAppError(cause, "无法保存 Provider");
      const field = errorField(error.code);
      if (field) {
        setError(field, { type: "server", message: error.message }, { shouldFocus: true });
      } else {
        setSubmissionError(error);
      }
      toast.error("无法保存 Provider");
    } finally {
      submissionInFlight.current = false;
      setSubmitting(false);
    }
  };

  const submitForm = () => {
    void handleSubmit(submit)();
  };

  const reloadAfterConflict = async () => {
    const error = await onReload();
    if (error) {
      setSubmissionError(error);
      return;
    }
    dismissAfterClearingSecret();
  };

  const retryPostSaveReload = async () => {
    if (!postSaveError || submissionInFlight.current) return;
    submissionInFlight.current = true;
    setSubmitting(true);
    const error = await onReload();
    if (error) {
      setPostSaveError(error);
    } else {
      dismissAfterClearingSecret();
    }
    submissionInFlight.current = false;
    setSubmitting(false);
  };

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open) requestDismiss(); }}>
        <DialogContent
          aria-describedby="provider-edit-description"
          className="provider-edit-dialog"
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
            submitForm();
          }}
        >
          <form
            className="provider-edit-form"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              submitForm();
            }}
          >
            <div className="provider-edit-form__body">
              <header className="provider-edit-heading">
                <DialogTitle>编辑 Provider</DialogTitle>
                <DialogDescription id="provider-edit-description">
                  只更新已支持的 Provider 字段。Provider ID 保持不变，现有 Direct API Key 不会显示。
                </DialogDescription>
              </header>

              <section className="provider-create-fields provider-edit-fields" aria-label="Provider 编辑字段">
                <EditField label="Provider ID" htmlFor="provider-edit-id">
                  <Input
                    id="provider-edit-id"
                    value={provider.id}
                    readOnly
                    autoComplete="off"
                    aria-describedby="provider-edit-id-note"
                  />
                  <p id="provider-edit-id-note" className="provider-edit-field-note">Stable ID 创建后不可修改。</p>
                </EditField>
                <EditField label="Base URL" htmlFor="provider-edit-base-url" error={errors.baseUrl?.message}>
                  <Input
                    id="provider-edit-base-url"
                    type="url"
                    inputMode="url"
                    autoComplete="url"
                    aria-invalid={Boolean(errors.baseUrl)}
                    {...register("baseUrl")}
                  />
                </EditField>
                <EditField label="默认协议（可选）" htmlFor="provider-edit-default-api" error={errors.defaultApi?.message}>
                  <Controller
                    control={control}
                    name="defaultApi"
                    render={({ field }) => (
                      <Select value={field.value || "inherit"} onValueChange={(value) => field.onChange(value === "inherit" ? "" : value)}>
                        <SelectTrigger id="provider-edit-default-api" aria-label="默认协议（可选）" className="provider-create-select">
                          <SelectValue placeholder="由模型指定" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="inherit">由模型指定</SelectItem>
                          {protocols.map((protocol) => <SelectItem key={protocol} value={protocol}>{protocol}</SelectItem>)}
                        </SelectContent>
                      </Select>
                    )}
                  />
                </EditField>
                <div className="provider-create-auth provider-edit-auth">
                  <span className="provider-create-auth__label">认证方式</span>
                  <Controller
                    control={control}
                    name="authMode"
                    render={({ field }) => (
                      <div className="provider-create-auth__choices" role="radiogroup" aria-label="认证方式">
                        <label className="provider-create-auth__choice">
                          <input
                            type="radio"
                            value="api-key"
                            checked={field.value === "api-key"}
                            aria-label="API Key 认证"
                            onChange={() => field.onChange("api-key")}
                          />
                          <span>API Key</span>
                        </label>
                        <label className="provider-create-auth__choice">
                          <input
                            type="radio"
                            value="none"
                            checked={field.value === "none"}
                            aria-label="无需认证"
                            onChange={requestNoAuthentication}
                          />
                          <span>无需认证</span>
                        </label>
                      </div>
                    )}
                  />
                </div>
                {values.authMode === "api-key" ? (
                  <EditField label="API Key" htmlFor="provider-edit-api-key" error={errors.apiKey?.message}>
                    <div className="provider-create-key-input">
                      <Input
                        id="provider-edit-api-key"
                        type={showApiKey ? "text" : "password"}
                        autoComplete="new-password"
                        placeholder="输入新的 API Key 以替换"
                        aria-invalid={Boolean(errors.apiKey)}
                        {...register("apiKey")}
                      />
                      <button type="button" aria-label={showApiKey ? "隐藏 API Key" : "显示 API Key"} onClick={() => setShowApiKey((visible) => !visible)}>
                        {showApiKey ? <Eye aria-hidden="true" /> : <EyeOff aria-hidden="true" />}
                      </button>
                    </div>
                    <p className="provider-edit-key-status">
                      {provider.hasApiKey ? "已配置。留空会保留当前密钥。" : "未配置。"}
                    </p>
                    {provider.hasApiKey ? (
                      <Button type="button" variant="secondary" className="provider-edit-delete-key" onClick={requestNoAuthentication}>
                        删除 API Key
                      </Button>
                    ) : null}
                  </EditField>
                ) : (
                  <p className="provider-edit-no-auth">无需认证 Provider 不会发送认证 Header。</p>
                )}
              </section>
              <p className="provider-create-security-note"><LockKeyhole aria-hidden="true" />Direct API Key 只在此次保存请求中使用，不会保存在应用状态或通知中。</p>

              {postSaveError ? (
                <section ref={feedbackRef} className="provider-create-submit-error" role="alert" aria-live="assertive">
                  <div>
                    <strong>Provider 已保存，但无法重新读取配置</strong>
                    <p>{postSaveError.message}</p>
                    <p>{postSaveError.action}</p>
                  </div>
                  <Button type="button" variant="secondary" disabled={submitting} onClick={() => void retryPostSaveReload()}>
                    {submitting ? "重新读取中…" : "重新读取"}
                  </Button>
                </section>
              ) : submissionError ? (
                <section ref={feedbackRef} className="provider-create-submit-error" role="alert" aria-live="assertive">
                  <div>
                    <strong>{submissionError.code === "models-hash-conflict" ? "配置冲突" : "无法保存 Provider"}</strong>
                    <p>{submissionError.message}</p>
                    <p>{submissionError.action}</p>
                  </div>
                  {submissionError.code === "models-hash-conflict" ? (
                    <Button type="button" variant="secondary" disabled={submitting} onClick={() => setConfirmConflictReload(true)}>
                      重新读取
                    </Button>
                  ) : null}
                </section>
              ) : null}
            </div>
            <footer className="provider-create-footer provider-edit-footer">
              <span />
              <div className="provider-create-footer__actions">
                <Button type="button" variant="secondary" disabled={submitting} onClick={requestDismiss}>取消</Button>
                <Button type="submit" disabled={!canSave || submitting || Boolean(postSaveError)} aria-busy={submitting}>
                  {submitting ? "保存中…" : "保存 Provider"}
                </Button>
              </div>
            </footer>
          </form>
        </DialogContent>
      </Dialog>

      {confirmNoAuth ? (
        <ConfirmDialog
          title="删除 Direct API Key？"
          cancelLabel="继续编辑"
          confirmLabel="删除并切换为无需认证"
          onCancel={() => setConfirmNoAuth(false)}
          onConfirm={() => {
            setConfirmNoAuth(false);
            applyNoAuthentication();
          }}
        >
          切换为无需认证会删除当前保存的 Direct API Key。此操作会在保存时创建备份。
        </ConfirmDialog>
      ) : null}
      {confirmConflictReload ? (
        <ConfirmDialog
          title="重新读取 Provider？"
          cancelLabel="继续编辑"
          confirmLabel="重新读取并丢弃修改"
          onCancel={() => setConfirmConflictReload(false)}
          onConfirm={() => {
            setConfirmConflictReload(false);
            void reloadAfterConflict();
          }}
        >
          重新读取会丢失当前未保存的修改。OMP Switch 不会自动合并外部修改。
        </ConfirmDialog>
      ) : null}
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
            dismissAfterClearingSecret();
          }}
        >
          离开后，这些修改将会丢失。
        </ConfirmDialog>
      ) : null}
    </>
  );
}

function EditField({
  label,
  htmlFor,
  error,
  children,
}: {
  label: string;
  htmlFor: string;
  error?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="provider-create-field provider-edit-field">
      <label htmlFor={htmlFor}>{label}</label>
      <div>
        {children}
        <p id={`${htmlFor}-error`} className="provider-create-field-error" aria-live="polite">{error ?? ""}</p>
      </div>
    </div>
  );
}

function keyIntent(
  values: ProviderEditValues,
  startedWithApiKey: boolean,
): EditCustomProviderInput["apiKey"] {
  if (values.authMode === "none") return startedWithApiKey ? { kind: "delete" } : { kind: "keep" };
  if (values.apiKey.trim()) return { kind: "replace", value: values.apiKey };
  return { kind: "keep" };
}

function errorField(code: string): "baseUrl" | "defaultApi" | "apiKey" | null {
  switch (code) {
    case "provider-base-url-invalid":
      return "baseUrl";
    case "provider-default-api-required":
      return "defaultApi";
    case "provider-api-key-invalid":
    case "provider-api-key-replacement-required":
    case "provider-auth-invalid":
      return "apiKey";
    default:
      return null;
  }
}
