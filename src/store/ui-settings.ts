import { create } from "zustand";
import type { Theme, UiSettings } from "../lib/tauri-client";

export type ModelSelection =
  | { kind: "none" }
  | { kind: "provider"; providerId: string }
  | { kind: "model"; providerId: string; modelId: string };

type HydrationState = "loading" | "ready" | "error";
type UiSettingsWithoutSelection = Omit<UiSettings, "selectedProviderId" | "selectedModelId">;

type UiSettingsState = UiSettingsWithoutSelection & {
  hydrationState: HydrationState;
  selection: ModelSelection;
  savedSelectionInvalid: boolean;
  setTheme(theme: Theme): void;
  setModelTestCostNoticeAccepted(accepted: boolean): void;
  beginHydration(): void;
  hydrate(settings: UiSettings): void;
  failHydration(): void;
  setSelection(selection: ModelSelection): void;
};

const NO_MODEL_SELECTION: ModelSelection = { kind: "none" };
const DEFAULT_UI_SETTINGS: UiSettingsWithoutSelection = {
  ompExecutablePath: null,
  theme: "system",
  modelTestCostNoticeAccepted: false,
};

function persistedModelSelection(providerId: string | null, modelId: string | null) {
  if (providerId === null) {
    return { selection: NO_MODEL_SELECTION, savedSelectionInvalid: modelId !== null };
  }
  return {
    selection: modelId === null
      ? { kind: "provider" as const, providerId }
      : { kind: "model" as const, providerId, modelId },
    savedSelectionInvalid: false,
  };
}

export function modelSelectionFields(selection: ModelSelection): Pick<UiSettings, "selectedProviderId" | "selectedModelId"> {
  switch (selection.kind) {
    case "none":
      return { selectedProviderId: null, selectedModelId: null };
    case "provider":
      return { selectedProviderId: selection.providerId, selectedModelId: null };
    case "model":
      return { selectedProviderId: selection.providerId, selectedModelId: selection.modelId };
  }
}

export const useUiSettings = create<UiSettingsState>((set) => ({
  ...DEFAULT_UI_SETTINGS,
  hydrationState: "loading",
  selection: NO_MODEL_SELECTION,
  savedSelectionInvalid: false,
  setTheme: (theme) => set({ theme }),
  setModelTestCostNoticeAccepted: (accepted) => set({ modelTestCostNoticeAccepted: accepted }),
  beginHydration: () => set({ ...DEFAULT_UI_SETTINGS, selection: NO_MODEL_SELECTION, savedSelectionInvalid: false, hydrationState: "loading" }),
  hydrate: ({ selectedProviderId, selectedModelId, ...settings }) => set({ ...settings, ...persistedModelSelection(selectedProviderId, selectedModelId), hydrationState: "ready" }),
  failHydration: () => set({ selection: NO_MODEL_SELECTION, savedSelectionInvalid: false, hydrationState: "error" }),
  setSelection: (selection) => set({ selection, savedSelectionInvalid: false }),
}));
