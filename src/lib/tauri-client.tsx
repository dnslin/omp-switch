import { invoke } from "@tauri-apps/api/core";
import { createContext, useContext, type PropsWithChildren } from "react";

export type StartupState = {
  kind: "omp-unavailable";
  message: string;
};

export type Theme = "light" | "dark" | "system";

export type UiSettings = {
  ompExecutablePath: string | null;
  theme: Theme;
  selectedProviderId: string | null;
  selectedModelId: string | null;
  costNoticeAccepted: boolean;
};

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
  getUiSettings(): Promise<UiSettings>;
  saveUiSettings(settings: UiSettings): Promise<UiSettings>;
}

export const tauriClient: TauriClient = {
  getStartupState: () => invoke<StartupState>("get_startup_state"),
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
