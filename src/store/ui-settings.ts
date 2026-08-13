import { create } from "zustand";
import type { Theme, UiSettings } from "../lib/tauri-client";

type UiSettingsState = UiSettings & {
  setTheme(theme: Theme): void;
};

export const useUiSettings = create<UiSettingsState>((set) => ({
  ompExecutablePath: null,
  theme: "system",
  selectedProviderId: null,
  selectedModelId: null,
  costNoticeAccepted: false,
  setTheme: (theme) => set({ theme }),
}));
