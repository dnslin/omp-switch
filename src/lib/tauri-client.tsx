import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { createContext, useContext, type PropsWithChildren } from "react";

export type ConfigurationFileStatus =
  | "normal"
  | "missing"
  | "read-only"
  | "alternate-only"
  | "canonical-with-alternate"
  | "legacy-json"
  | "parse-error"
  | "unsafe";
export type TargetConfigurationStatus =
  | "writable"
  | "read-only"
  | "creation-required"
  | "migration-required"
  | "parse-error"
  | "unsafe";
export type ConfigurationFileDiscovery = {
  canonicalPath: string;
  resolvedPath: string | null;
  status: ConfigurationFileStatus;
};
export type ConfigurationIssue = {
  filePath: string;
  line: number | null;
  column: number | null;
  message: string;
};
export type TargetConfigurationDiscovery = {
  path: string;
  resolvedPath: string | null;
  status: TargetConfigurationStatus;
  writable: boolean;
  models: ConfigurationFileDiscovery;
  config: ConfigurationFileDiscovery;
  recoveryNotice: string | null;
  createPaths: string[];
  discoveryToken: string;
  warnings: string[];
  issue: ConfigurationIssue | null;
};
export type OverviewState = "normal" | "empty" | "read-only";
export type OverviewApi = "openai-completions" | "openai-responses" | "anthropic-messages" | "google-generative-ai";

export type CreateCustomProviderInput = {
  openedModelsHash: string;
  provider: {
    id: string;
    baseUrl: string;
    defaultApi?: OverviewApi;
    authMode: "api-key" | "none";
    apiKey?: string;
  };
  firstModel: {
    id: string;
    name: string;
    api?: OverviewApi;
    reasoning: boolean;
    input: Array<"text" | "image">;
    contextWindow: number;
    maxTokens: number;
  };
};

export type CreateCustomProviderResult = { providerId: string; modelId: string };

export type DirectApiKeyIntent =
  | { kind: "keep" }
  | { kind: "replace"; value: string }
  | { kind: "delete" };

export type EditCustomProviderInput = {
  openedModelsHash: string;
  providerId: string;
  baseUrl: string;
  defaultApi?: OverviewApi;
  authMode: "api-key" | "none";
  apiKey: DirectApiKeyIntent;
};


export type EditCustomProviderResult = { providerId: string };
export type ModelStatus = "normal" | "incomplete" | "read-only";

export type ModelDefinitionFields = {
  id: string;
  name: string;
  api?: OverviewApi;
  reasoning: boolean;
  input: Array<"text" | "image">;
  contextWindow: number;
  maxTokens: number;
};

export type ModelEditFields = Omit<ModelDefinitionFields, "id">;

export type CreateModelInput = {
  openedModelsHash: string;
  providerId: string;
  model: ModelDefinitionFields;
};

export type EditModelInput = {
  openedModelsHash: string;
  providerId: string;
  modelId: string;
  model: ModelEditFields;
};

export type DeleteModelInput = {
  openedModelsHash: string;
  openedConfigHash: string;
  providerId: string;
  modelId: string;
};

export type ModelMutationResult = { providerId: string; modelId: string };

export type ModelTestInput = { providerId: string; modelId: string };
export type ModelTestResult = {
  success: boolean;
  providerId: string;
  modelId: string;
  protocol: OverviewApi;
  latencyMs: number;
  status?: number;
  message: string;
  errorCode?: string;
};
export type ModelTestState = {
  running: boolean;
  providerId: string | null;
  modelId: string | null;
  result: ModelTestResult | null;
};
export type OverviewModel = {
  providerId: string;
  id: string;
  name: string | null;
  effectiveApi: OverviewApi | null;
  apiSource: OverviewApiSource | null;
  hasBaseUrlOverride: boolean;
  input: OverviewInput[];
  reasoning: boolean | null;
  contextWindow: number | null;
  maxTokens: number | null;
  complete: boolean;
  unsupportedProtocol: boolean;
  status: ModelStatus;
  editable: boolean;
  referenceCount: number;
  referencePaths: string[];
  readOnlyReason: string | null;
};
export type OverviewAuthMode = "api-key" | "none" | "unsupported";
export type OverviewApiSource = "provider" | "model";
export type OverviewRoleStatus = "configured" | "unconfigured" | "provider-missing" | "model-missing" | "incomplete" | "unsupported" | "advanced";
export type OverviewInput = "text" | "image" | "unsupported";
export type OverviewFile = {
  canonicalPath: string;
  resolvedPath: string | null;
  status: ConfigurationFileStatus;
  contentHash: string | null;
};
export type OverviewProviderClassification = "custom" | "built-in-override" | "advanced" | "unsupported" | "unavailable";
export type OverviewProvider = {
  id: string;
  name: string | null;
  baseUrl: string | null;
  defaultApi: OverviewApi | null;
  authMode: OverviewAuthMode;
  hasApiKey: boolean;
  modelCount: number;
  classification: OverviewProviderClassification;
  editable: boolean;
  readOnlyReason: string | null;
  models: OverviewModel[];
};
export type OverviewRole = { id: string; status: OverviewRoleStatus; selector: string | null; providerId: string | null; modelId: string | null; thinkingLevel: SupportedThinkingLevel | null };

export type SupportedThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto";
export type ModelRoleChange =
  | { kind: "set"; roleId: string; providerId: string; modelId: string; thinkingLevel?: SupportedThinkingLevel }
  | { kind: "create"; roleId: string; providerId: string; modelId: string; thinkingLevel?: SupportedThinkingLevel }
  | { kind: "rename"; roleId: string; newRoleId: string; providerId: string; modelId: string; thinkingLevel?: SupportedThinkingLevel }
  | { kind: "clear"; roleId: string }
  | { kind: "delete"; roleId: string };
export type SaveModelRolesInput = { openedConfigHash: string; changes: ModelRoleChange[] };
export type SaveModelRolesResult = { changedRoleCount: number };

export type OverviewDto = {
  state: OverviewState;
  omp: { status: "connected"; executablePath: string; version: string };
  targetConfiguration: TargetConfigurationDiscovery;
  files: { models: OverviewFile; config: OverviewFile };
  counts: { providerCount: number; modelCount: number; roleCount: number };
  providers: OverviewProvider[];
  models: OverviewModel[];
  roles: OverviewRole[];
  rolesEditable: boolean;
  rolesAssignable: boolean;
  rolesReadOnlyReason: string | null;
  emptyReason: string | null;
  nextAction: string | null;
  readOnlyReason: string | null;
};

export type TargetInitializationExpectation = Pick<TargetConfigurationDiscovery, "createPaths" | "discoveryToken">;
export type StartupState =
  | { kind: "detecting" }
  | { kind: "omp-unavailable"; message: string }
  | { kind: "invalid-executable"; executablePath: string; message: string; diagnosticCode: string }
  | { kind: "version-failed"; executablePath: string; message: string; diagnosticCode: string; exitCode: number | null; stderr: string }
  | { kind: "config-path-failed"; executablePath: string; version: string; message: string; diagnosticCode: string; exitCode: number | null; stderr: string }
  | { kind: "omp-ready"; executablePath: string; version: string; targetConfiguration: TargetConfigurationDiscovery; previousTargetConfiguration: string | null; requiresConfirmation: boolean };
export type OverviewLoad = {
  startupState: StartupState;
  overview: OverviewDto | null;
  error: AppError | null;
};

export type Theme = "light" | "dark" | "system";

export type UiSettings = {
  ompExecutablePath: string | null;
  theme: Theme;
  selectedProviderId: string | null;
  selectedModelId: string | null;
  modelTestCostNoticeAccepted: boolean;
};

export type UiSettingsUpdate = Omit<UiSettings, "ompExecutablePath" | "modelTestCostNoticeAccepted">;

export type AppError = {
  code: string;
  message: string;
  action: string;
};

export function asAppError(error: unknown, fallbackMessage: string): AppError {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    "action" in error &&
    typeof error.code === "string" &&
    typeof error.message === "string" &&
    typeof error.action === "string"
  ) {
    return { code: error.code, message: error.message, action: error.action };
  }
  return {
    code: "client-error",
    message: fallbackMessage,
    action: "请重试；如果问题持续，请查看脱敏日志。",
  };
}

export interface TauriClient {
  getStartupState(): Promise<StartupState>;
  getOverviewLoad(): Promise<OverviewLoad>;
  createCustomProvider(input: CreateCustomProviderInput): Promise<CreateCustomProviderResult>;
  editCustomProvider(input: EditCustomProviderInput): Promise<EditCustomProviderResult>;
  createModel(input: CreateModelInput): Promise<ModelMutationResult>;
  editModel(input: EditModelInput): Promise<ModelMutationResult>;
  deleteModel(input: DeleteModelInput): Promise<ModelMutationResult>;
  saveModelRoles(input: SaveModelRolesInput): Promise<SaveModelRolesResult>;
  testModel(input: ModelTestInput): Promise<ModelTestResult>;
  cancelModelTest(): Promise<boolean>;
  getModelTestState(): Promise<ModelTestState>;
  detectOmp(): Promise<StartupState>;
  selectOmpExecutable(): Promise<string | null>;
  validateSelectedOmp(executablePath: string): Promise<StartupState>;
  confirmSelectedOmp(executablePath: string): Promise<void>;
  initializeTargetConfiguration(executablePath: string, expectation: TargetInitializationExpectation): Promise<StartupState>;
  openTargetConfigurationDirectory(executablePath: string): Promise<void>;
  getUiSettings(): Promise<UiSettings>;
  saveUiSettings(settings: UiSettingsUpdate): Promise<UiSettings>;
  acceptModelTestCostNotice(): Promise<UiSettings>;
}
export const tauriClient: TauriClient = {
  getStartupState: () => invoke<StartupState>("get_startup_state"),
  getOverviewLoad: () => invoke<OverviewLoad>("get_overview_load"),
  createCustomProvider: (input) => invoke<CreateCustomProviderResult>("create_custom_provider", { input }),
  editCustomProvider: (input) => invoke<EditCustomProviderResult>("edit_custom_provider", { input }),
  createModel: (input) => invoke<ModelMutationResult>("create_model", { input }),
  editModel: (input) => invoke<ModelMutationResult>("edit_model", { input }),
  deleteModel: (input) => invoke<ModelMutationResult>("delete_model", { input }),
  saveModelRoles: (input) => invoke<SaveModelRolesResult>("save_model_roles", { input }),
  testModel: (input) => invoke<ModelTestResult>("test_model", { input }),
  cancelModelTest: () => invoke<boolean>("cancel_model_test"),
  getModelTestState: () => invoke<ModelTestState>("get_model_test_state"),
  detectOmp: () => invoke<StartupState>("detect_omp"),
  selectOmpExecutable: async () => {
    const selected = await open({ multiple: false, directory: false, title: "选择 OMP 可执行文件" });
    return typeof selected === "string" ? selected : null;
  },
  validateSelectedOmp: (executablePath) => invoke<StartupState>("validate_selected_omp", { executablePath }),
  confirmSelectedOmp: async (executablePath) => { await invoke("confirm_selected_omp", { executablePath }); },
  initializeTargetConfiguration: (executablePath, expectation) => invoke<StartupState>("initialize_target_configuration", { executablePath, expectation }),
  openTargetConfigurationDirectory: async (executablePath) => { await invoke("open_target_configuration_directory", { executablePath }); },
  getUiSettings: () => invoke<UiSettings>("get_ui_settings"),
  saveUiSettings: (settings) => invoke<UiSettings>("save_ui_settings", { settings }),
  acceptModelTestCostNotice: () => invoke<UiSettings>("accept_model_test_cost_notice"),
};



const TauriClientContext = createContext<TauriClient | null>(null);

export function TauriClientProvider({
  client,
  children,
}: PropsWithChildren<{ client: TauriClient }>) {
  return <TauriClientContext.Provider value={client}>{children}</TauriClientContext.Provider>;
}

export function useTauriClient(): TauriClient {
  const client = useContext(TauriClientContext);
  if (!client) throw new Error("TauriClientProvider is required");
  return client;
}
