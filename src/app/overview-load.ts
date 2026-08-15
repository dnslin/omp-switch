import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router";

import {
  asAppError,
  useTauriClient,
  type AppError,
  type OverviewDto,
  type StartupState,
} from "../lib/tauri-client";
import { overviewShellStatus, startupShellStatus, type ShellStatus } from "./omp-presentation";

export type OverviewLoadCopy = {
  readonly missingOverview: AppError;
  readonly requestFailure: string;
};

export function useOverviewLoad(copy: OverviewLoadCopy) {
  const client = useTauriClient();
  const navigate = useNavigate();
  const [data, setData] = useState<OverviewDto | null>(null);
  const [startupState, setStartupState] = useState<StartupState | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const requestId = useRef(0);

  const reload = useCallback(async () => {
    const currentRequest = ++requestId.current;
    setLoading(true);
    setData(null);
    setError(null);
    try {
      const result = await client.getOverviewLoad();
      if (currentRequest !== requestId.current) return;
      setStartupState(result.startupState);
      if (result.startupState.kind === "omp-ready" && result.startupState.requiresConfirmation) {
        navigate("/setup", { replace: true });
        return;
      }
      if (result.error) {
        setError(result.error);
      } else if (result.overview) {
        setData(result.overview);
      } else {
        setError(copy.missingOverview);
      }
    } catch (cause: unknown) {
      if (currentRequest !== requestId.current) return;
      setStartupState(null);
      setError(asAppError(cause, copy.requestFailure));
    } finally {
      if (currentRequest === requestId.current) setLoading(false);
    }
  }, [client, copy, navigate]);

  useEffect(() => {
    void reload();
    return () => { requestId.current += 1; };
  }, [reload]);

  const shellStatus: ShellStatus = data
    ? overviewShellStatus(data)
    : startupState
      ? startupShellStatus(startupState)
      : error
        ? { title: "OMP 状态不可用", path: "配置目录不可用", status: "请重新读取 OMP", tone: "warning" }
        : { title: "正在检测 OMP", path: "配置目录检测中", status: "请稍候", tone: "warning" };

  return { data, startupState, error, loading, reload, shellStatus };
}
