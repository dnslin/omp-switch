import { create } from "zustand";
import type { AppError, ModelTestResult, ModelTestState, ModelTestTerminal } from "../lib/tauri-client";

type ModelTestStore = ModelTestState & {
  error: AppError | null;
  generation: number;
  needsOverviewRefresh: boolean;
  source: "local" | "remote";
  prepareHydration(): void;
  begin(providerId: string, modelId: string): boolean;
  finish(result: ModelTestResult): void;
  fail(terminal?: ModelTestTerminal): void;
  setError(error: AppError | null): void;
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
  terminal: null,
  error: null,
  generation: 0,
  needsOverviewRefresh: false,
  source: "remote",
  prepareHydration: () => {
    const current = get();
    if (current.running && current.source === "local") return;
    set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: false, source: "remote", result: null, terminal: null, error: null }));
  },
  begin: (providerId, modelId) => {
    if (get().running) return false;
    set((state) => ({ generation: state.generation + 1, source: "local", running: true, providerId, modelId, terminal: null, error: null }));
    return true;
  },
  finish: (result) => set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: true, source: "local", running: false, providerId: null, modelId: null, result, terminal: null, error: null })),
  fail: (terminal) => set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: false, source: "remote", running: false, providerId: null, modelId: null, result: null, terminal: terminal ?? null })),
  setError: (error) => set({ error }),
  hydrate: (state, generation) => {
    const current = get();
    if (current.generation !== generation || current.source === "local") return;
    const completedResultArrived = !state.running && (current.running || !sameResult(current.result, state.result));
    set({ ...state, generation, needsOverviewRefresh: current.needsOverviewRefresh || completedResultArrived, source: "remote" });
  },
  recoverRemote: () => {
    set((state) => ({ generation: state.generation + 1, needsOverviewRefresh: false, source: "remote", running: true, providerId: null, modelId: null, error: null }));
    return get().generation;
  },
  reconcile: (state, generation) => {
    const current = get();
    if (current.generation !== generation || (current.running && current.source === "local")) return;
    const preserveRemotePolling = current.source === "remote" && state.running;
    set((currentState) => ({ ...state, generation: preserveRemotePolling ? currentState.generation : currentState.generation + 1, needsOverviewRefresh: false, source: "remote" }));
  },
}));
