import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { createContext, useContext, type PropsWithChildren } from "react";

export type ConfigurationFileStatus = "normal" | "missing" | "read-only";
export type TargetAccess = { writable: boolean; modelsYml: ConfigurationFileStatus; configYml: ConfigurationFileStatus };
export type StartupState =
  | { kind: "detecting" }
  | { kind: "omp-unavailable"; message: string }
  | { kind: "invalid-executable"; executablePath: string; message: string; diagnosticCode: string }
  | { kind: "version-failed"; executablePath: string; message: string; diagnosticCode: string; exitCode: number | null; stderr: string }
  | { kind: "config-path-failed"; executablePath: string; version: string; message: string; diagnosticCode: string; exitCode: number | null; stderr: string }
  | { kind: "omp-ready"; executablePath: string; version: string; targetConfiguration: string; previousTargetConfiguration: string | null; targetAccess: TargetAccess; requiresConfirmation: boolean };

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
  detectOmp(): Promise<StartupState>;
  selectOmpExecutable(): Promise<string | null>;
  validateSelectedOmp(executablePath: string): Promise<StartupState>;
  confirmSelectedOmp(executablePath: string): Promise<void>;
  getUiSettings(): Promise<UiSettings>;
  saveUiSettings(settings: UiSettingsUpdate): Promise<UiSettings>;
}

export const tauriClient: TauriClient = {
  getStartupState: () => invoke<StartupState>("get_startup_state"),
  detectOmp: () => invoke<StartupState>("detect_omp"),
  selectOmpExecutable: async () => {
    const selected = await open({ multiple: false, directory: false, title: "选择 OMP 可执行文件" });
    return typeof selected === "string" ? selected : null;
  },
  validateSelectedOmp: (executablePath) => invoke<StartupState>("validate_selected_omp", { executablePath }),
  confirmSelectedOmp: async (executablePath) => { await invoke("confirm_selected_omp", { executablePath }); },
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
