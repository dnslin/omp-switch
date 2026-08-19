import { CircleAlert, LockKeyhole, MoreHorizontal, Pencil, Star, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBeforeUnload, useBlocker } from "react-router";
import { toast } from "sonner";

import { Button, ConfirmDialog, PageTitle, SearchInput, StatusIndicator } from "../components/ui";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../components/ui/select";
import {
  asAppError,
  type AppError,
  type ModelRoleChange,
  type OverviewDto,
  type OverviewModel,
  type OverviewRole,
  type OverviewRoleStatus,
  type SupportedThinkingLevel,
  useTauriClient,
} from "../lib/tauri-client";
import { MainShell } from "./MainShell";
import { useOverviewLoad } from "./overview-load";
import type { ShellStatus } from "./omp-presentation";

const BUILT_IN_ROLES = [
  ["default", "通用对话的默认角色"],
  ["smol", "快速响应，简洁回答"],
  ["slow", "深入思考，全面分析"],
  ["vision", "图像理解与分析"],
  ["plan", "计划与任务分解"],
  ["designer", "设计与创意构思"],
  ["commit", "代码提交与变更说明"],
  ["tiny", "极简输出，节省 Token"],
  ["task", "复杂任务处理"],
  ["advisor", "专业建议与决策支持"],
] as const;

const THINKING_LEVELS: Array<{ value: SupportedThinkingLevel | ""; label: string }> = [
  { value: "", label: "模型默认" },
  { value: "off", label: "off" },
  { value: "minimal", label: "minimal" },
  { value: "low", label: "low" },
  { value: "medium", label: "medium" },
  { value: "high", label: "high" },
  { value: "xhigh", label: "xhigh" },
  { value: "max", label: "max" },
  { value: "auto", label: "auto" },
];

const NONE_VALUE = "__none__";
const SELECT_VALUE_PREFIX = "role-value:";

function encodeSelectValue(value: string) {
  return value ? `${SELECT_VALUE_PREFIX}${value}` : NONE_VALUE;
}

function decodeSelectValue(value: string) {
  return value === NONE_VALUE ? "" : value.startsWith(SELECT_VALUE_PREFIX) ? value.slice(SELECT_VALUE_PREFIX.length) : value;
}
function foldStableId(value: string) {
  return value.replace(/[A-Z]/g, (character) => String.fromCharCode(character.charCodeAt(0) + 32));
}


function sameStableId(left: string, right: string) {
  return foldStableId(left) === foldStableId(right);
}
function selectValueForId(value: string, options: ReadonlyArray<{ id: string }>) {
  return options.find((option) => sameStableId(option.id, value))?.id ?? value;
}

type RoleDraft = {
  id: string;
  originalId: string | null;
  builtin: boolean;
  description: string;
  selector: string | null;
  providerId: string;
  modelId: string;
  thinkingLevel: SupportedThinkingLevel | "";
  status: OverviewRoleStatus;
};

type RoleEditorState = {
  mode: "create" | "edit" | "rename";
  row: RoleDraft | null;
};

type RoleEditorValues = {
  id: string;
  providerId: string;
  modelId: string;
  thinkingLevel: SupportedThinkingLevel | "";
};

type RoleStatusView = {
  label: string;
  tone: "success" | "neutral" | "warning" | "danger";
};

const roleLoadCopy = {
  missingOverview: {
    code: "roles-missing-overview",
    message: "OMP 没有返回模型角色所需的数据。",
    action: "请重新读取；如果问题持续，请查看脱敏日志。",
  },
  requestFailure: "无法读取模型角色",
};

function roleStatusView(status: OverviewRoleStatus): RoleStatusView {
  switch (status) {
    case "configured": return { label: "正常", tone: "success" };
    case "unconfigured": return { label: "未配置", tone: "neutral" };
    case "provider-missing": return { label: "Provider 不存在", tone: "warning" };
    case "model-missing": return { label: "模型不存在", tone: "warning" };
    case "incomplete": return { label: "模型配置不完整", tone: "warning" };
    case "unsupported": return { label: "不支持协议", tone: "warning" };
    case "advanced": return { label: "高级配置，只读", tone: "danger" };
  }
}


function selectorFor(providerId: string, modelId: string, thinkingLevel: SupportedThinkingLevel | "") {
  if (!providerId || !modelId) return null;
  return `${providerId}/${modelId}${thinkingLevel ? `:${thinkingLevel}` : ""}`;
}

function roleFromOverview(
  definition: { id: string; description: string; builtin: boolean },
  overviewRole: OverviewRole | undefined,
): RoleDraft {
  const providerId = overviewRole?.providerId ?? "";
  const modelId = overviewRole?.modelId ?? "";
  const thinkingLevel = overviewRole?.thinkingLevel ?? "";
  return {
    id: definition.id,
    originalId: definition.id,
    builtin: definition.builtin,
    description: definition.description,
    selector: selectorFor(providerId, modelId, thinkingLevel) ?? overviewRole?.selector ?? null,
    providerId,
    modelId,
    thinkingLevel,
    status: overviewRole?.status ?? "unconfigured",
  };
}

function initialRows(data: OverviewDto): RoleDraft[] {
  const persisted = new Map(data.roles.map((role) => [role.id, role]));
  const rows = BUILT_IN_ROLES.map(([id, description]) => roleFromOverview({ id, description, builtin: true }, persisted.get(id)));
  for (const role of data.roles) {
    if (BUILT_IN_ROLES.some(([id]) => id === role.id)) continue;
    rows.push(roleFromOverview({ id: role.id, description: "自定义模型角色", builtin: false }, role));
  }
  return rows;
}

function modelStatusForSelection(data: OverviewDto, providerId: string, modelId: string): OverviewRoleStatus {
  if (!providerId) return "unconfigured";
  const provider = data.providers.find((candidate) => sameStableId(candidate.id, providerId));
  if (!provider) return "provider-missing";
  if (!modelId) return "unconfigured";
  const model = provider.models.find((candidate) => sameStableId(candidate.id, modelId));
  if (!model) return "model-missing";
  if (model.status === "read-only" || !model.editable) return model.readOnlyReason?.includes("不支持的协议") ? "unsupported" : model.complete ? "configured" : "incomplete";
  if (model.status === "incomplete" || !model.complete) return "incomplete";
  return "configured";
}

function roleIsDirty(row: RoleDraft, initial: RoleDraft | undefined) {
  return !initial
    || row.id !== initial.id
    || row.selector !== initial.selector
    || row.providerId !== initial.providerId
    || row.modelId !== initial.modelId
    || row.thinkingLevel !== initial.thinkingLevel;
}
function hasWhitespaceOrControl(value: string) {
  return Array.from(value).some((character) => /\s|\p{Cc}/u.test(character));
}


function isValidRoleId(value: string) {
  const trimmed = value.trim();
  return trimmed.length > 0
    && trimmed === value
    && !Array.from(value).some((character) => character === "/" || character === "," || hasWhitespaceOrControl(character));
}

function buildChanges(initial: RoleDraft[], draft: RoleDraft[]): ModelRoleChange[] {
  const changes: ModelRoleChange[] = [];
  const draftByOriginalId = new Map(draft.filter((row) => row.originalId).map((row) => [row.originalId!, row]));
  for (const original of initial) {
    const current = draftByOriginalId.get(original.id);
    if (!current) {
      if (!original.builtin) changes.push({ kind: "delete", roleId: original.id });
      continue;
    }
    if (current.id !== original.id) {
      if (!current.providerId || !current.modelId) continue;
      changes.push({
        kind: "rename",
        roleId: original.id,
        newRoleId: current.id,
        providerId: current.providerId,
        modelId: current.modelId,
        ...(current.thinkingLevel ? { thinkingLevel: current.thinkingLevel } : {}),
      });
      continue;
    }
    if (current.selector === original.selector) continue;
    if (!current.selector) {
      if (current.providerId || current.modelId || current.thinkingLevel) continue;
      changes.push({ kind: "clear", roleId: current.id });
    } else {
      changes.push({
        kind: "set",
        roleId: current.id,
        providerId: current.providerId,
        modelId: current.modelId,
        ...(current.thinkingLevel ? { thinkingLevel: current.thinkingLevel } : {}),
      });
    }
  }
  for (const row of draft.filter((candidate) => candidate.originalId === null)) {
    if (!row.selector) continue;
    changes.push({
      kind: "create",
      roleId: row.id,
      providerId: row.providerId,
      modelId: row.modelId,
      ...(row.thinkingLevel ? { thinkingLevel: row.thinkingLevel } : {}),
    });
  }
  return changes;
}

function isSimpleRoleSelector(providerId: string, modelId: string, thinkingLevel: SupportedThinkingLevel | "") {
  const selector = selectorFor(providerId, modelId, thinkingLevel);
  if (!selector) return false;
  if (selector.startsWith("@") || selector.includes(",") || hasWhitespaceOrControl(selector)) return false;
  const separator = selector.indexOf("/");
  if (separator <= 0) return false;
  const parsedProviderId = selector.slice(0, separator);
  const modelWithThinking = selector.slice(separator + 1);
  const thinkingSeparator = modelWithThinking.lastIndexOf(":");
  let parsedModelId = modelWithThinking;
  let parsedThinkingLevel = "" as SupportedThinkingLevel | "";
  if (thinkingSeparator >= 0) {
    const possibleThinking = modelWithThinking.slice(thinkingSeparator + 1);
    if (THINKING_LEVELS.some(({ value }) => value !== "" && value === possibleThinking)) {
      parsedModelId = modelWithThinking.slice(0, thinkingSeparator);
      parsedThinkingLevel = possibleThinking as SupportedThinkingLevel;
    }
  }
  return parsedProviderId === providerId && parsedModelId === modelId && parsedThinkingLevel === thinkingLevel;
}

function availableProviders(data: OverviewDto) {
  return data.providers.filter((provider) => provider.editable && provider.models.some((model) => model.editable && model.status === "normal" && model.complete && isSimpleRoleSelector(provider.id, model.id, "")));
}

function availableModels(data: OverviewDto, providerId: string): OverviewModel[] {
  const provider = data.providers.find((candidate) => sameStableId(candidate.id, providerId));
  return provider?.models.filter((model) => model.editable && model.status === "normal" && model.complete && isSimpleRoleSelector(provider.id, model.id, "")) ?? [];
}

function isAssignableModel(data: OverviewDto, providerId: string, modelId: string) {
  return Boolean(providerId && modelId && availableModels(data, providerId).some((model) => sameStableId(model.id, modelId)));
}

function RoleSelect({
  label,
  value,
  disabled,
  onValueChange,
  children,
}: {
  label: string;
  value: string;
  disabled: boolean;
  onValueChange(value: string): void;
  children: React.ReactNode;
}) {
  return (
    <Select value={encodeSelectValue(value)} onValueChange={(selectedValue) => onValueChange(decodeSelectValue(selectedValue))} disabled={disabled}>
      <SelectTrigger aria-label={label} className="roles-select">
        <SelectValue placeholder="未配置" />
      </SelectTrigger>
      <SelectContent>{children}</SelectContent>
    </Select>
  );
}

function RoleEditorDialog({
  editor,
  data,
  existingRoleIds,
  onCancel,
  onSave,
  onDirtyChange,
}: {
  editor: RoleEditorState;
  data: OverviewDto;
  existingRoleIds: Set<string>;
  onCancel(): void;
  onSave(values: RoleEditorValues): void;
  onDirtyChange(dirty: boolean): void;
}) {
  const initialValues = useMemo<RoleEditorValues>(() => ({
    id: editor.row?.id ?? "",
    providerId: editor.row?.providerId ?? "",
    modelId: editor.row?.modelId ?? "",
    thinkingLevel: editor.row?.thinkingLevel ?? "",
  }), [editor]);
  const [values, setValues] = useState<RoleEditorValues>(initialValues);
  const [error, setError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const dirty = values.id !== initialValues.id
    || values.providerId !== initialValues.providerId
    || values.modelId !== initialValues.modelId
    || values.thinkingLevel !== initialValues.thinkingLevel;
  useEffect(() => {
    setValues(initialValues);
    setError(null);
    setConfirmDiscard(false);
  }, [initialValues]);
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  const requestCancel = () => {
    if (dirty) {
      setConfirmDiscard(true);
      return;
    }
    onCancel();
  };

  const providers = availableProviders(data);
  const models = availableModels(data, values.providerId);
  const selectedModelAssignable = isAssignableModel(data, values.providerId, values.modelId);
  const title = editor.mode === "create" ? "新增自定义角色" : editor.mode === "rename" ? "改名自定义角色" : "编辑自定义角色";
  const submitLabel = editor.mode === "create" ? "添加" : "保存";
  const save = () => {
    if (!isValidRoleId(values.id)) {
      setError("角色名称不能为空，且不能包含空白、/、逗号或控制字符。");
      return;
    }
    const isCurrentRoleId = values.id === editor.row?.id || values.id === editor.row?.originalId;
    if ((editor.mode === "create" || editor.mode === "rename") && existingRoleIds.has(values.id) && !isCurrentRoleId) {
      setError("角色名称已存在或与内置角色重名。");
      return;
    }
    if (!selectedModelAssignable) {
      setError("请选择普通、完整的 Provider 和 Model definition。");
      return;
    }
    onSave(values);
  };

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open) requestCancel(); }}>
        <DialogContent
          className="roles-editor-dialog"
          aria-describedby="roles-editor-description"
          onEscapeKeyDown={(event) => { event.preventDefault(); requestCancel(); }}
          onPointerDownOutside={(event) => { event.preventDefault(); requestCancel(); }}
          onKeyDown={(event) => {
            if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "s") return;
            event.preventDefault();
            save();
          }}
        >
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription id="roles-editor-description">只写入 Simple role selector 和 Supported Thinking Level。</DialogDescription>
          <form className="roles-editor-form" noValidate onSubmit={(event) => { event.preventDefault(); save(); }}>
            <div className="roles-editor-fields">
              <label className="roles-editor-field">
                <span>角色名称</span>
                <Input aria-label="角色名称" value={values.id} readOnly={editor.mode === "edit"} onChange={(event) => setValues((current) => ({ ...current, id: event.target.value }))} />
              </label>
              <label className="roles-editor-field">
                <span>Provider</span>
                <RoleSelect label="Provider" value={selectValueForId(values.providerId, providers)} disabled={false} onValueChange={(value) => setValues((current) => ({ ...current, providerId: value, modelId: "" }))}>
                  <SelectItem value={NONE_VALUE}>请选择 Provider</SelectItem>
                  {providers.map((provider) => <SelectItem key={provider.id} value={encodeSelectValue(provider.id)}>{provider.id}</SelectItem>)}
                </RoleSelect>
              </label>
              <label className="roles-editor-field">
                <span>模型</span>
                <RoleSelect label="模型" value={selectValueForId(values.modelId, models)} disabled={!values.providerId} onValueChange={(value) => setValues((current) => ({ ...current, modelId: value }))}>
                  <SelectItem value={NONE_VALUE}>请选择模型</SelectItem>
                  {models.map((model) => <SelectItem key={model.id} value={encodeSelectValue(model.id)}>{model.id}</SelectItem>)}
                </RoleSelect>
              </label>
              <label className="roles-editor-field">
                <span>Thinking</span>
                <RoleSelect label="Thinking" value={values.thinkingLevel} disabled={!values.providerId || !values.modelId || !selectedModelAssignable} onValueChange={(value) => setValues((current) => ({ ...current, thinkingLevel: value as SupportedThinkingLevel | "" }))}>
                  {THINKING_LEVELS.map(({ value, label }) => <SelectItem key={value || NONE_VALUE} value={encodeSelectValue(value)}>{label}</SelectItem>)}
                </RoleSelect>
              </label>
              {error ? <p className="roles-editor-error" role="alert">{error}</p> : null}
            </div>
            <footer className="roles-editor-footer">
              <Button type="button" variant="secondary" onClick={requestCancel}>取消</Button>
              <Button type="submit">{submitLabel}</Button>
            </footer>
          </form>
        </DialogContent>
      </Dialog>
      {confirmDiscard ? (
        <ConfirmDialog
          title="有未保存的修改"
          cancelLabel="继续编辑"
          confirmLabel="放弃修改"
          onCancel={() => setConfirmDiscard(false)}
          onConfirm={() => { setConfirmDiscard(false); onCancel(); }}
        >
          离开后，这些修改将会丢失。
        </ConfirmDialog>
      ) : null}
    </>
  );
}


function RolesSkeleton() {
  return (
    <div className="roles-skeleton" aria-label="正在读取模型角色" role="status">
      <div className="roles-skeleton-header" />
      <div className="roles-skeleton-search" />
      <div className="roles-skeleton-table" />
    </div>
  );
}

export function ModelRolesPage() {
  const client = useTauriClient();
  const { data, error, loading, reload, shellStatus, startupState } = useOverviewLoad(roleLoadCopy);
  const [draft, setDraft] = useState<RoleDraft[]>([]);
  const [search, setSearch] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [editor, setEditor] = useState<RoleEditorState | null>(null);
  const [editorDirty, setEditorDirty] = useState(false);
  const [deleteRole, setDeleteRole] = useState<RoleDraft | null>(null);
  const [clearBuiltIns, setClearBuiltIns] = useState(false);
  const [reloadConfirmation, setReloadConfirmation] = useState(false);
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [initialDraft, setInitialDraft] = useState<RoleDraft[]>([]);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const baseRows = useMemo(() => data ? initialRows(data) : [], [data]);

  useEffect(() => {
    if (!data) return;
    setDraft(baseRows);
    setInitialDraft(baseRows);
    setSaveError(null);
  }, [baseRows, data]);

  const currentRows = draft.length > 0 ? draft : baseRows;
  const originalRows = initialDraft.length > 0 ? initialDraft : baseRows;
  const initialById = useMemo(() => new Map(originalRows.map((row) => [row.id, row])), [originalRows]);
  const pendingChanges = useMemo(() => buildChanges(originalRows, currentRows), [currentRows, originalRows]);
  const stagedDirty = pendingChanges.length > 0;
  const draftDirty = currentRows.some((row) => roleIsDirty(row, initialById.get(row.originalId ?? row.id)));
  const dirty = stagedDirty || draftDirty || editorDirty;
  const blocker = useBlocker(dirty);
  useBeforeUnload((event) => {
    if (!dirty) return;
    event.preventDefault();
    event.returnValue = "";
  });

  const advancedRoles = data?.roles.filter((role) => role.status === "advanced") ?? [];
  const advancedLocked = advancedRoles.length > 0;
  const rolesEditable = Boolean(data?.rolesEditable) && !advancedLocked;
  const rolesAssignable = Boolean(data?.rolesAssignable) && rolesEditable;
  const configHash = data?.files.config.contentHash ?? null;
  const canEdit = rolesEditable && !saving;
  const providers = data ? availableProviders(data) : [];
  const canAssign = rolesAssignable && !saving && providers.length > 0;
  const hasEmptyNewRole = currentRows.some((row) => !row.builtin && row.originalId === null && !row.selector);
  const canSave = stagedDirty && canEdit && !hasEmptyNewRole;
  const rowsForView = currentRows;
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const visibleRows = rowsForView.filter((row) => [row.id, row.description, row.providerId, row.modelId, roleStatusView(row.status).label].some((value) => value.toLocaleLowerCase().includes(normalizedSearch)));
  const existingRoleIds = useMemo(() => new Set([...originalRows.map((row) => row.id), ...currentRows.map((row) => row.id)]), [currentRows, originalRows]);

  const updateRole = useCallback((roleId: string, update: Partial<RoleDraft>) => {
    setDraft((current) => (current.length > 0 ? current : baseRows).map((row) => row.id === roleId ? { ...row, ...update } : row));
  }, [baseRows]);


  const selectProvider = useCallback((row: RoleDraft, value: string) => {
    const providerId = value;
    updateRole(row.id, {
      providerId,
      modelId: "",
      thinkingLevel: "",
      selector: null,
      status: modelStatusForSelection(data!, providerId, ""),
    });
  }, [data, updateRole]);

  const selectModel = useCallback((row: RoleDraft, value: string) => {
    const modelId = value;
    updateRole(row.id, {
      modelId,
      selector: selectorFor(row.providerId, modelId, row.thinkingLevel),
      status: modelStatusForSelection(data!, row.providerId, modelId),
    });
  }, [data, updateRole]);

  const selectThinking = useCallback((row: RoleDraft, value: string) => {
    const thinkingLevel = value as SupportedThinkingLevel | "";
    updateRole(row.id, {
      thinkingLevel,
      selector: selectorFor(row.providerId, row.modelId, thinkingLevel),
      status: modelStatusForSelection(data!, row.providerId, row.modelId),
    });
  }, [data, updateRole]);

  const save = useCallback(async () => {
    if (!data || !configHash || !canSave) return;
    setSaving(true);
    setSaveError(null);
    try {
      await client.saveModelRoles({ openedConfigHash: configHash, changes: pendingChanges });
      const reloadError = await reload();
      if (reloadError) {
        setSaveError(reloadError);
        return;
      }
      toast.success("模型角色已保存");
    } catch (cause: unknown) {
      setSaveError(asAppError(cause, "保存模型角色失败"));
    } finally {
      setSaving(false);
    }
  }, [canSave, client, configHash, data, pendingChanges, reload]);
  const openTargetDirectory = useCallback(async () => {
    const executablePath = startupState?.kind === "omp-ready" ? startupState.executablePath : data?.omp.executablePath;
    if (!executablePath) {
      toast.error("无法打开配置目录");
      return;
    }
    try {
      await client.openTargetConfigurationDirectory(executablePath);
    } catch (cause: unknown) {
      const appError = asAppError(cause, "无法打开配置目录");
      toast.error(appError.message);
    }
  }, [client, data?.omp.executablePath, startupState]);
  const requestReload = useCallback(() => {
    if (dirty) {
      setReloadConfirmation(true);
      return;
    }
    void reload();
  }, [dirty, reload]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || editor || deleteRole || clearBuiltIns || reloadConfirmation || blocker.state === "blocked") return;
      const key = event.key.toLowerCase();
      if (key === "f") {
        event.preventDefault();
        searchInputRef.current?.focus();
      } else if (key === "s") {
        event.preventDefault();
        if (canSave) void save();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [blocker.state, canSave, clearBuiltIns, deleteRole, editor, reloadConfirmation, save]);

  const clearAllBuiltIns = () => {
    setDraft((current) => (current.length > 0 ? current : baseRows).map((row) => row.builtin ? { ...row, selector: null, providerId: "", modelId: "", thinkingLevel: "", status: "unconfigured" } : row));
    setClearBuiltIns(false);
  };

  const saveEditor = (values: RoleEditorValues) => {
    if (!editor) return;
    const selector = selectorFor(values.providerId, values.modelId, values.thinkingLevel);
    const base = editor.row ?? {
      id: values.id,
      originalId: null,
      builtin: false,
      description: "自定义模型角色",
      selector: null,
      providerId: "",
      modelId: "",
      thinkingLevel: "" as const,
      status: "unconfigured" as const,
    };
    const next: RoleDraft = {
      ...base,
      id: values.id,
      providerId: values.providerId,
      modelId: values.modelId,
      thinkingLevel: values.thinkingLevel,
      selector,
      status: modelStatusForSelection(data!, values.providerId, values.modelId),
    };
    setDraft((current) => {
      const rows = current.length > 0 ? current : baseRows;
      return editor.mode === "create" ? [...rows, next] : rows.map((row) => row.id === editor.row?.id ? next : row);
    });
    setEditorDirty(false);
    setEditor(null);
  };

  const shell: ShellStatus = shellStatus.status === "配置目录可读写"
    ? { ...shellStatus, status: "config.yml 将在保存时自动备份" }
    : shellStatus;
  return (
    <MainShell status={shell} contentClassName="page-content--roles">
      <div className="roles-page" aria-busy={loading}>
        {loading ? <RolesSkeleton /> : error ? (
          <section className="roles-error" role="alert" aria-live="assertive">
            <CircleAlert aria-hidden="true" />
            <div><h1>无法读取模型角色</h1><p>{error.message}</p><p>{error.action}</p></div>
            <Button type="button" variant="secondary" onClick={requestReload}>重新读取</Button>
          </section>
        ) : !data ? (
          <section className="roles-error" role="alert"><div><h1>模型角色不可用</h1><p>OMP 没有返回角色配置。</p></div></section>
        ) : (
          <>
            <header className="roles-header">
              <PageTitle title="角色" description="OMP 的不同任务选择默认模型。" />
              <div className="roles-header-actions">
                {dirty ? <span className="roles-dirty-status"><span className="status-dot" aria-hidden="true" />有未保存的修改</span> : null}
                <Button type="button" variant="secondary" className="roles-more-button" aria-label="更多角色操作" disabled={!canEdit} onClick={() => setClearBuiltIns(true)}><MoreHorizontal aria-hidden="true" size={20} /></Button>
                <Button type="button" disabled={!canSave} onClick={() => void save()}>{saving ? "保存中…" : "保存修改"}</Button>
                <Button type="button" variant="secondary" disabled={!canAssign} onClick={() => setEditor({ mode: "create", row: null })}>新增自定义角色</Button>
              </div>
            </header>
            {!rolesEditable ? (
              <section className="roles-lock-banner" role="status" aria-live="polite">
                <LockKeyhole aria-hidden="true" />
                <div>
                  <strong>角色配置为只读</strong>
                  {advancedLocked ? <p>以下角色使用当前版本不支持的高级选择器：{advancedRoles.map((role) => role.id).join("、")}</p> : null}
                  <p>{advancedLocked ? "为避免部分覆盖，整个角色页面暂时不能修改。" : data.rolesReadOnlyReason ?? "当前配置业务结构无法安全编辑；角色值仍会原样保留。"}</p>
                  <Button type="button" variant="secondary" onClick={() => void openTargetDirectory()}>打开配置目录</Button>
                </div>
              </section>
            ) : null}
            {saveError ? <section className="roles-save-error" role="alert"><div><strong>{saveError.code === "config-hash-conflict" ? "配置冲突" : "无法保存模型角色"}</strong><p>{saveError.message}</p><p>{saveError.action}</p></div><Button type="button" variant="secondary" onClick={requestReload}>重新读取</Button></section> : null}
            <SearchInput ref={searchInputRef} name="role-search" aria-label="搜索角色" className="roles-search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索角色…" />
            <div className="roles-table-scroll">
              <table className="roles-table">
                <colgroup><col className="roles-col-role" /><col className="roles-col-description" /><col className="roles-col-provider" /><col className="roles-col-model" /><col className="roles-col-thinking" /><col className="roles-col-status" /><col className="roles-col-actions" /></colgroup>
                <thead><tr><th scope="col">角色</th><th scope="col">说明</th><th scope="col">Provider</th><th scope="col">模型</th><th scope="col">Thinking</th><th scope="col">状态</th><th scope="col">操作</th></tr></thead>
                <tbody>
                  {visibleRows.map((row) => {
                    const initial = initialById.get(row.originalId ?? row.id);
                    const dirtyRow = roleIsDirty(row, initial);
                    const status: RoleStatusView = dirtyRow ? { label: "待保存", tone: "warning" } : roleStatusView(row.status);
                    const providerChoices: Array<{ id: string; editable: boolean }> = providers.map((provider) => ({ id: provider.id, editable: provider.editable }));
                    if (row.providerId && !providerChoices.some((provider) => sameStableId(provider.id, row.providerId))) {
                      providerChoices.push({ id: row.providerId, editable: false });
                    }
                    const modelChoices: Array<{ id: string; editable: boolean }> = availableModels(data, row.providerId).map((model) => ({ id: model.id, editable: model.editable }));
                    if (row.modelId && !modelChoices.some((model) => sameStableId(model.id, row.modelId))) {
                      modelChoices.push({ id: row.modelId, editable: false });
                    }
                    const rowAssignmentDisabled = !canAssign;
                    const rowActionDisabled = !canEdit;
                    return (
                      <tr key={`${row.originalId ?? "new"}-${row.id}`} className={dirtyRow ? "roles-row--dirty" : undefined}>
                        <td><div className="roles-role-cell"><span>{row.id}</span>{row.builtin && row.id === "default" ? <Star aria-label="默认内置角色" className="roles-default-star" size={17} /> : null}{!row.builtin ? <span className="roles-custom-tag">自定义</span> : null}</div></td>
                        <td>{row.description}</td>
                        <td><RoleSelect label={`Provider ${row.id}`} value={selectValueForId(row.providerId, providerChoices)} disabled={rowAssignmentDisabled} onValueChange={(value) => selectProvider(row, value)}><SelectItem value={NONE_VALUE}>未配置</SelectItem>{providerChoices.map((provider) => <SelectItem key={provider.id} value={encodeSelectValue(provider.id)} disabled={!provider.editable}>{provider.id}</SelectItem>)}</RoleSelect></td>
                        <td><RoleSelect label={`模型 ${row.id}`} value={selectValueForId(row.modelId, modelChoices)} disabled={rowAssignmentDisabled || !row.providerId} onValueChange={(value) => selectModel(row, value)}><SelectItem value={NONE_VALUE}>未配置</SelectItem>{modelChoices.map((model) => <SelectItem key={model.id} value={encodeSelectValue(model.id)} disabled={!model.editable}>{model.id}</SelectItem>)}</RoleSelect></td>
                        <td><RoleSelect label={`Thinking ${row.id}`} value={row.thinkingLevel} disabled={rowAssignmentDisabled || !isAssignableModel(data, row.providerId, row.modelId)} onValueChange={(value) => selectThinking(row, value)}>{THINKING_LEVELS.map(({ value, label }) => <SelectItem key={value || NONE_VALUE} value={encodeSelectValue(value)}>{label}</SelectItem>)}</RoleSelect></td>
                        <td><StatusIndicator tone={status.tone}>{status.label}</StatusIndicator></td>
                        <td><div className="roles-actions-cell">
                          {row.builtin ? <Button type="button" variant="secondary" className="roles-clear-button" disabled={rowActionDisabled || !row.selector} onClick={() => updateRole(row.id, { selector: null, providerId: "", modelId: "", thinkingLevel: "", status: "unconfigured" })}>清除</Button> : <>
                            <Button type="button" variant="secondary" className="roles-more-button roles-row-more" aria-label={`角色操作 ${row.id}`} aria-expanded={openMenu === row.id} disabled={rowActionDisabled} onClick={() => setOpenMenu((current) => current === row.id ? null : row.id)}><MoreHorizontal aria-hidden="true" size={19} /></Button>
                            {openMenu === row.id ? <div className="roles-row-menu" role="menu">
                              <Button type="button" variant="secondary" role="menuitem" disabled={!canAssign} onClick={() => { setOpenMenu(null); setEditor({ mode: "edit", row }); }}><Pencil aria-hidden="true" size={14} />编辑</Button>
                              <Button type="button" variant="secondary" role="menuitem" disabled={!canAssign} onClick={() => { setOpenMenu(null); setEditor({ mode: "rename", row }); }}>改名</Button>
                              <Button type="button" variant="secondary" role="menuitem" className="roles-danger-action" disabled={!canEdit} onClick={() => { setOpenMenu(null); setDeleteRole(row); }}><Trash2 aria-hidden="true" size={14} />删除</Button>
                            </div> : null}
                          </>}
                        </div></td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            {editor && data ? <RoleEditorDialog editor={editor} data={data} existingRoleIds={existingRoleIds} onCancel={() => { setEditorDirty(false); setEditor(null); }} onSave={saveEditor} onDirtyChange={setEditorDirty} /> : null}
            {deleteRole ? <ConfirmDialog title="删除自定义角色？" confirmLabel="删除角色" onCancel={() => setDeleteRole(null)} onConfirm={() => { setDraft((current) => current.filter((row) => row.id !== deleteRole.id)); setDeleteRole(null); }}><p>将删除角色 {deleteRole.id} 及其模型选择器。</p><p>Provider 和模型配置不会被删除；保存前仍可通过放弃修改撤销。</p></ConfirmDialog> : null}
            {clearBuiltIns ? <ConfirmDialog title="清除全部内置角色？" confirmLabel="清除" onCancel={() => setClearBuiltIns(false)} onConfirm={clearAllBuiltIns}><p>将清除 {BUILT_IN_ROLES.map(([id]) => id).join("、")} 的模型选择器。</p><p>自定义角色不受影响。本操作只修改表单，仍需点击“保存修改”。</p></ConfirmDialog> : null}
          </>
        )}
        {reloadConfirmation ? <ConfirmDialog title="重新读取配置？" cancelLabel="取消" confirmLabel="重新读取" onCancel={() => setReloadConfirmation(false)} onConfirm={() => { setReloadConfirmation(false); void reload(); }}><p>重新读取会丢弃当前未保存的角色修改。</p><p>如需保留草稿，请先取消并完成保存。</p></ConfirmDialog> : null}
        {blocker.state === "blocked" ? <ConfirmDialog title="有未保存的修改" confirmLabel="放弃修改" onCancel={() => blocker.reset()} onConfirm={() => blocker.proceed()}><p>离开后，这些修改将会丢失。</p></ConfirmDialog> : null}
      </div>
    </MainShell>
  );
}
