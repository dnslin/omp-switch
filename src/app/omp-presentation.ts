import type { ConfigurationFileStatus, OverviewDto, StartupState, TargetConfigurationStatus } from "../lib/tauri-client";

export type StatusTone = "success" | "warning" | "danger";
export type RowStatus = { label: string; tone: StatusTone };
export type ShellStatus = { title: string; path: string; status: string; tone: StatusTone };

type TargetConfigurationStatusView = { label: string; tone: StatusTone };

const FILE_STATUS_VIEW: Record<ConfigurationFileStatus, RowStatus> = {
  normal: { label: "正常", tone: "success" },
  missing: { label: "缺失", tone: "warning" },
  "read-only": { label: "只读", tone: "warning" },
  "alternate-only": { label: ".yaml 只读", tone: "warning" },
  "canonical-with-alternate": { label: "正常 · 有 .yaml", tone: "warning" },
  "legacy-json": { label: "旧 JSON", tone: "warning" },
  "parse-error": { label: "格式错误", tone: "danger" },
  unsafe: { label: "不安全", tone: "danger" },
};

const TARGET_CONFIGURATION_STATUS_VIEW: Record<TargetConfigurationStatus, TargetConfigurationStatusView> = {
  writable: { label: "配置目录可读写", tone: "success" },
  "read-only": { label: "配置目录只读", tone: "warning" },
  "creation-required": { label: "需要创建配置文件", tone: "warning" },
  "migration-required": { label: "需要由 OMP 迁移", tone: "warning" },
  "parse-error": { label: "配置文件格式错误", tone: "danger" },
  unsafe: { label: "配置目录不安全", tone: "danger" },
};

export function fileStatusView(status: ConfigurationFileStatus): RowStatus {
  return FILE_STATUS_VIEW[status];
}

export function targetConfigurationStatusView(status: TargetConfigurationStatus): TargetConfigurationStatusView {
  return TARGET_CONFIGURATION_STATUS_VIEW[status];
}

export function startupShellStatus(state: StartupState): ShellStatus {
  switch (state.kind) {
    case "detecting":
      return { title: "正在检测 OMP", path: "配置目录检测中", status: "请稍候", tone: "warning" };
    case "omp-unavailable":
      return { title: "OMP 不可用", path: "配置目录不可用", status: state.message, tone: "warning" };
    case "invalid-executable":
    case "version-failed":
    case "config-path-failed":
      return { title: "OMP 不可用", path: state.executablePath, status: state.message, tone: "danger" };
    case "omp-ready": {
      const targetView = targetConfigurationStatusView(state.targetConfiguration.status);
      return {
        title: `OMP 已连接  ·  ${formatOmpVersion(state.version)}`,
        path: state.targetConfiguration.resolvedPath ?? state.targetConfiguration.path,
        status: targetView.label,
        tone: targetView.tone,
      };
    }
  }
}

export function overviewShellStatus(data: OverviewDto): ShellStatus {
  const filesNeedAttention = [data.files.models, data.files.config].some(
    (file) => file.contentHash === null || file.status !== "normal",
  );
  const targetView = targetConfigurationStatusView(data.targetConfiguration.status);
  const tone = targetView.tone === "danger"
    ? "danger"
    : data.state === "read-only" || filesNeedAttention || targetView.tone === "warning"
      ? "warning"
      : "success";
  return {
    title: `OMP 已连接  ·  ${formatOmpVersion(data.omp.version)}`,
    path: data.targetConfiguration.resolvedPath ?? data.targetConfiguration.path,
    status: targetView.tone === "danger" ? targetView.label : filesNeedAttention ? "配置文件需注意" : targetView.label,
    tone,
  };
}

function formatOmpVersion(version: string) {
  const normalized = version.trim();
  return /^(?:v|omp\/)/i.test(normalized) ? normalized : `v${normalized}`;
}
