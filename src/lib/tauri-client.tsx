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
export type OverviewApiSource = "provider" | "model";
export type OverviewAuthMode = "api-key" | "none" | "unsupported";
export type OverviewRoleStatus = "configured" | "unconfigured" | "provider-missing" | "model-missing" | "incomplete" | "advanced";
export type OverviewInput = "text" | "image" | "unsupported";
export type OverviewFile = {
  canonicalPath: string;
  resolvedPath: string | null;
  status: ConfigurationFileStatus;
  contentHash: string | null;
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
  editable: boolean;
  readOnlyReason: string | null;
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
export type OverviewRole = { id: string; status: OverviewRoleStatus; selector: string | null };
export type OverviewDto = {
  state: OverviewState;
  omp: { status: "connected"; executablePath: string; version: string };
  targetConfiguration: TargetConfigurationDiscovery;
  files: { models: OverviewFile; config: OverviewFile };
  counts: { providerCount: number; modelCount: number; roleCount: number };
  providers: OverviewProvider[];
  models: OverviewModel[];
  roles: OverviewRole[];
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
  costNoticeAccepted: boolean;
};

export type UiSettingsUpdate = Omit<UiSettings, "ompExecutablePath">;

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
  detectOmp(): Promise<StartupState>;
  selectOmpExecutable(): Promise<string | null>;
  validateSelectedOmp(executablePath: string): Promise<StartupState>;
  confirmSelectedOmp(executablePath: string): Promise<void>;
  initializeTargetConfiguration(executablePath: string, expectation: TargetInitializationExpectation): Promise<StartupState>;
  openTargetConfigurationDirectory(executablePath: string): Promise<void>;
  getUiSettings(): Promise<UiSettings>;
  saveUiSettings(settings: UiSettingsUpdate): Promise<UiSettings>;
}


export const tauriClient: TauriClient = {
  getStartupState: () => invoke<StartupState>("get_startup_state"),
  getOverviewLoad: () => invoke<OverviewLoad>("get_overview_load"),
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
