import { create } from "zustand";
import type { ModelTestResult, ModelTestState } from "../lib/tauri-client";

type ModelTestStore = ModelTestState & {
  generation: number;
  needsOverviewRefresh: boolean;
  source: "local" | "remote";
  prepareHydration(): void;
  begin(providerId: string, modelId: string): boolean;
  finish(result: ModelTestResult): void;
  fail(): void;
  hydrate(state: ModelTestState, generation: number): void;
  recoverRemote(): number;
  reconcile(state: ModelTestState, generation: number): void;
};

function sameResult(left: ModelTestResult | null, right: ModelTestResult | null): boolean {
  return left?.success === right?.success
    && left?.providerId === right?.providerId
    && left?.modelId === right?.modelId
    && left?.protocol === right?.protocol
    && left?.latencyMs === right?.latencyMs
    && left?.status === right?.status
    && left?.message === right?.message
    && left?.errorCode === right?.errorCode;
}

export const useModelTestStore = create<ModelTestStore>((set, get) => ({
  running: false,
  providerId: null,
  modelId: null,
  result: null,
  generation: 0,
  needsOverviewRefresh: false,
  source: "remote",
  prepareHydration: () => {
    const current = get();
    if (current.running && current.source === "local") return;
    set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: false, source: "remote", result: null }));
  },
  begin: (providerId, modelId) => {
    if (get().running) return false;
    set((state) => ({ generation: state.generation + 1, source: "local", running: true, providerId, modelId }));
    return true;
  },
  finish: (result) => set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: true, source: "local", running: false, providerId: null, modelId: null, result })),
  fail: () => set((state) => ({ generation: state.generation + 1, source: "local", running: false, providerId: null, modelId: null })),
  hydrate: (state, generation) => {
    const current = get();
    if (current.generation !== generation || current.source === "local") return;
    const completedResultArrived = !state.running && (current.running || !sameResult(current.result, state.result));
    set({ ...state, needsOverviewRefresh: current.needsOverviewRefresh || completedResultArrived, source: "remote" });
  },
  recoverRemote: () => {
    set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: false, source: "remote", running: true, providerId: null, modelId: null }));
    return get().generation;
  },
  reconcile: (state, generation) => {
    const current = get();
    if (current.generation !== generation || (current.running && current.source === "local")) return;
    set((currentState) => ({ ...state, generation: currentState.generation + 1, needsOverviewRefresh: false, source: "remote" }));
  },
}));
