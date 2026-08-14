import { create } from "zustand";
import type { Theme, UiSettings } from "../lib/tauri-client";

type HydrationState = "loading" | "ready" | "error";

type UiSettingsState = UiSettings & {
  hydrationState: HydrationState;
  setTheme(theme: Theme): void;
  beginHydration(): void;
  hydrate(settings: UiSettings): void;
  failHydration(): void;
  setSelection(providerId: string | null, modelId: string | null): void;
};

const DEFAULT_UI_SETTINGS: UiSettings = {
  ompExecutablePath: null,
  theme: "system",
  selectedProviderId: null,
  selectedModelId: null,
  costNoticeAccepted: false,
};

export const useUiSettings = create<UiSettingsState>((set) => ({
  ...DEFAULT_UI_SETTINGS,
  hydrationState: "loading",
  setTheme: (theme) => set({ theme }),
  beginHydration: () => set({ ...DEFAULT_UI_SETTINGS, hydrationState: "loading" }),
  hydrate: (settings) => set({ ...settings, hydrationState: "ready" }),
  failHydration: () => set({ selectedProviderId: null, selectedModelId: null, hydrationState: "error" }),
  setSelection: (providerId, modelId) => set({ selectedProviderId: providerId, selectedModelId: modelId }),
}));
