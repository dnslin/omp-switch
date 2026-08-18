import { create } from "zustand";
import type { ModelTestResult, ModelTestState } from "../lib/tauri-client";

type ModelTestStore = ModelTestState & {
  generation: number;
  source: "local" | "remote";
  prepareHydration(): void;
  begin(providerId: string, modelId: string): boolean;
  finish(result: ModelTestResult): void;
  fail(): void;
  hydrate(state: ModelTestState, generation: number): void;
};

export const useModelTestStore = create<ModelTestStore>((set, get) => ({
  running: false,
  providerId: null,
  modelId: null,
  result: null,
  generation: 0,
  source: "remote",
  prepareHydration: () => {
    const current = get();
    if (current.running && current.source === "local") return;
    set((state) => ({ generation: state.generation + 1, source: "remote" }));
  },
  begin: (providerId, modelId) => {
    if (get().running) return false;
    set((state) => ({ generation: state.generation + 1, source: "local", running: true, providerId, modelId }));
    return true;
  },
  finish: (result) => set((state) => ({ generation: state.generation + 1, source: "local", running: false, providerId: null, modelId: null, result })),
  fail: () => set((state) => ({ generation: state.generation + 1, source: "local", running: false, providerId: null, modelId: null })),
  hydrate: (state, generation) => {
    const current = get();
    if (current.generation !== generation || current.source === "local") return;
    set({ ...state, source: "remote" });
  },
}));
