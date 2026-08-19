# OMP Switch MVP

> 版本：v0.1  
> 状态：已确认，待实现  
> 产品形态：跨平台桌面应用

## 1. 文档职责

本文档定义 MVP 的产品范围、发布阻断项和验收标准。

文档关系：

```text
MVP → PRD → Flow
```

- `mvp.md` 定义范围、发布阻断项和验收标准。
- `prd.md` 只能在 MVP 范围内细化产品行为和业务规则。
- `flow.md` 只能根据 MVP 和 PRD 细化页面状态、文案和交互。
- PRD 或 Flow 不得静默扩大、缩小或改变上游规则。
- 范围发生变化时，三份文档必须同步修改。
- 批准的视觉权威文件为 `designs/omp-switch.pen`。实现必须按该文件中的 Foundations、Components 和对应页面画板 1:1 还原；Flow 负责交互约束，不替代视觉稿。

Issue #5 的 Setup 页面存在一项经产品负责人确认的明确例外：保留 `.pen` Foundations、Components、检测表格、状态行、间距和操作区，但移除最外层整页卡片的背景、边框、圆角、阴影和内边距。该修订后的基准以 `.artifacts/issue-5/implementation-1536x1024.png` 和 `.artifacts/issue-5/responsive-cardless-1100x720.png` 为准，并覆盖 `02 Page / Setup Success` 对最外层卡片装饰的要求；其他页面和 Setup 内部组件仍按 `.pen` 还原。

## 2. 产品定义

OMP Switch 是 OMP 配置的安全结构化编辑器。

MVP 只管理 OMP 自定义 Provider、Provider 下的模型，以及 `config.yml` 中的模型角色。它不替代 OMP、不启动 OMP、不管理终端、会话或项目工作目录。

核心流程：

```text
检测 OMP
→ 获取权威配置目录
→ 读取并验证配置
→ 管理自定义 Provider 和模型
→ 手动测试模型
→ 管理模型角色
→ 安全写回 OMP 配置
```

MVP 是可公开使用的正式版本。安全写入、备份、冲突检测、密钥保护、兼容保留、三平台验收和自动化测试都是发布阻断项。

## 3. 发布平台

以下平台必须在发布前完成真实运行验收：

- macOS 13 Ventura 及以上：Intel、Apple Silicon。
- Windows 10 22H2、Windows 11：x64。
- Ubuntu 22.04 LTS、Ubuntu 24.04 LTS：x64。

其他 Linux 发行版、Windows ARM64 和 Linux ARM64 不属于 MVP 的正式支持矩阵。

界面采用平台中立的桌面 UI。三个平台共享相同的信息架构；文件选择器、路径显示和目录打开使用平台原生能力。除操作系统原生窗口控件、文件选择器和路径/快捷键平台差异外，最终视觉必须与 `designs/omp-switch.pen` 1:1 一致。

## 4. MVP 范围

### 4.1 包含

- OMP 可执行文件检测和手动选择。
- OMP 版本读取。
- 通过 `omp config path` 获取权威配置目录。
- `models.yml` 和 `config.yml` 的读取、验证和定点写入。
- 缺失目录和缺失 `.yml` 文件的确认创建。
- 自定义 Provider 与其首个模型的原子创建，以及已有 Provider 管理。
- 直接文本 API Key 和无认证模式。
- Provider 默认协议与模型协议覆盖。
- 四种模型协议的连接测试。
- 模型管理。
- 内置角色和自定义角色管理。
- Thinking Level 管理。
- 外部修改 Hash 冲突检测。
- 自动备份。
- 跨文件可恢复事务。
- 应用设置、主题、日志和错误处理。
- Rust、前端和三平台手工验收。

### 4.2 不包含

- 启动 OMP。
- 终端、会话或工作目录管理。
- 项目级 `.omp/config.yml` 管理。
- Profile 列表、创建、切换、重命名或删除 UI。
- Provider 预设或模型市场。
- 模型自动发现。
- OAuth。
- `.env` 或环境变量凭据配置。
- `!command` 凭据的创建、查看、编辑或执行。
- 自定义 Header 的创建或编辑。
- OMP 高级 Provider 配置编辑。
- OMP 高级模型配置编辑。
- 不支持协议的编辑或测试。
- Provider ID 或 Model ID 重命名。
- YAML 注释、空行、缩进、标量样式和原始键顺序保真。
- JSON 配置迁移。
- `.yaml` 文件写入。
- 实时文件监听。
- 备份浏览或恢复页面。
- 自动更新、数据库、用户账号和云同步。

## 5. 配置目标与文件发现

### 5.1 OMP 检测

应用支持：

1. 使用用户已保存的 OMP 可执行文件路径。
2. 从系统 `PATH` 查找 `omp`。
3. 用户手动选择 OMP 可执行文件。

检测执行：

```text
omp --version
omp config path
```

`omp config path` 的成功输出是权威配置目录。OMP Switch 静默运行该命令，但文档和错误详情必须如实说明：该命令由 OMP 自身实现，会初始化 OMP Settings、访问 `agent.db`，并可能在缺少主 YAML 时执行 OMP 自身的旧设置迁移。

命令失败时：

- 不猜测配置目录。
- 不回退到硬编码路径。
- 配置功能不可用。
- 显示脱敏错误、退出码和重试入口。
- 允许重新检测或重新选择 OMP。

写入许可不由 OMP 版本号单独决定。每次写入都必须通过当前目标目录、文件结构、业务字段、未触及路径、临时文件重解析和引用完整性检查；任一检查失败时只读。OMP 版本只用于诊断和兼容报告。

实现约束（issue #4）：Rust 暴露固定的检测、无副作用手动验证和显式确认意图。React 不能提交参数或其他命令；检测结果包含目标访问性和规范 `.yml` 文件状态，缺失文件不会自动创建。

成功状态下重新检测时，React 保留当前检测结果，使用覆盖整个应用窗口的半透明模糊遮罩，并在窗口中心单独显示 Dot Matrix 加载面板和“正在重新检测 OMP”文案。加载反馈至少展示 1200ms；期间禁用重新检测和进入应用，检测完成后再一次性呈现新结果。

### 5.2 支持的文件

MVP 只写入：

```text
models.yml
config.yml
```

规则：

- 只有 `models.yaml` 或 `config.yaml` 时，配置只读并提示扩展名不受支持。
- `.yml` 和 `.yaml` 同时存在时，使用 `.yml`，并警告 `.yaml` 不会被修改。
- 不自动重命名、迁移或删除 `.yaml`。
- 缺失文件只创建规范 `.yml` 名称。
- 检测到旧 JSON 配置且缺少受支持 YAML 时，要求用户先通过 OMP 完成官方迁移。

### 5.3 缺失目录和文件

权威配置目录不存在时：

- 显示将创建的目录和文件。
- 用户确认后创建目录和最小有效 `.yml`。

最小有效结构：

```yaml
# models.yml
providers: {}
```

```yaml
# config.yml
modelRoles: {}
```

文件编码为 UTF-8，使用 LF 换行；创建后立即重新解析验证。

- 不覆盖已存在文件。
- 中间路径类型异常、权限不足或目标不安全时中止。

配置目录或文件是符号链接、junction 或 reparse point 时：

- 解析并显示真实目标。
- 在真实目标所在目录创建临时文件并执行替换。
- 链接循环、目标变化、权限边界异常或无法确认目标时拒绝写入。
- OMP Switch 不创建新的链接。

实现状态（issue #5）：Rust 启动状态现已返回 Target configuration 的规范路径、真实目标、文件分类、创建清单、警告和 YAML 错误位置；React 首次设置页据此展示创建确认、`.yaml` 只读、旧 JSON 迁移、解析错误和不安全目标状态。创建前会重新比对用户确认的文件清单和现有父路径真实目标；最小配置通过同目录临时文件验证、无覆盖提交、失败回滚和重新发现完成，回滚不完整时会明确报告残留风险。本工单不读取配置业务投影，也不提供编辑。
实现状态（issue #6）：Rust 概览读取保留 `models.yml` 与 `config.yml` 的完整 YAML 树、原始 Hash、真实目标路径和安全领域投影；未知根路径、Provider/Model 未知字段及其他配置路径留在后端快照，不进入 DTO。React 概览通过类型化 Tauri IPC 呈现统计、文件同步状态和快速测试摘要，Direct API Key 仅以存在性元数据存在于 DTO/前端状态；Loading、Empty、Error、Normal、Read-only 共用 `.pen` 骨架和 token。Provider/Model 选择按所属 Provider 内完整 pair hydration、验证和持久化；失效选择精确清理，快速变更串行保存完整 UI settings，读取失败时保持会话内选择但不写盘。安全投影中的只读和不完整条目仍可查看摘要，模型测试继续禁用。


## 6. 安全结构化写入

### 6.1 数据来源

- Provider 和模型：`models.yml.providers`。
- 角色：`config.yml.modelRoles`。
- OMP 配置文件是上述业务数据的唯一来源。
- 应用设置只保存 OMP 可执行文件路径、主题和轻量界面状态。

### 6.2 定点修改

`models.yml`：

- 保留完整解析树。
- 只修改 `providers` 下用户明确操作的 Provider、模型和支持字段。
- 不整体重建 `providers`。
- 不修改根节点其他字段、其他 Provider 或目标对象中的未知字段。
- 显式删除完整 Provider 或模型时，才允许删除对应完整节点。

`config.yml`：

- 保留完整解析树。
- 只修改 `modelRoles` 中用户明确操作的角色键。
- 不整体重建 `modelRoles`。
- 不修改其他配置路径。
- 显式删除角色时，只删除对应角色键。

写回后重新解析，并逐值比较所有未触及路径。任何未触及值发生变化时，写入失败。

MVP 保证数据语义和未触及值，不保证 YAML 注释、空行、缩进、标量样式和原始键顺序。

### 6.3 单文件写入

```text
锁定并重新读取
→ 比较打开时 Hash
→ 创建备份
→ 在最新解析树上定点修改
→ 验证完整配置
→ 写入同目录临时文件并 fsync
→ 重新解析临时文件
→ 比较未触及路径
→ 原子替换
→ 重新读取并刷新 UI
```

### 6.4 跨文件事务

删除 Provider 或模型可能同时修改 `models.yml` 和 `config.yml`。此类操作使用一个可恢复事务：

1. 锁定并重新读取两个文件。
2. 比较两个文件的打开时 Hash。
3. 为两个文件创建同一事务 ID 的备份。
4. 生成并验证全部临时文件。
5. 写入持久事务清单，记录原始 Hash、最终 Hash、备份和目标路径。
6. 依次原子替换目标文件。
7. 全部成功后删除事务清单。

启动时发现未完成事务：

- 如果所有目标都匹配事务清单的最终 Hash，只完成事务清理。
- 否则先保存当前现场副本，再从同一事务的全部备份恢复所有目标文件。
- 不允许只恢复其中一个文件。

### 6.5 外部修改

MVP 不运行实时文件监听器。

保存前重新读取目标文件并比较内容 Hash。Hash 不一致时停止保存，提示重新加载；不提供自动合并。

## 7. Provider 管理

### 7.1 可编辑自定义 Provider

MVP 可编辑的自定义 Provider 必须同时满足：

- `models` 是非空列表；产品不保存空 Provider。
- Provider ID 和 Provider/Model ID 不与当前 OMP bundled catalog 发生不区分大小写的冲突。
- 仅包含 MVP 支持的普通 Provider 字段。
- 不使用不受支持的协议、OAuth、命令凭据、自定义 Header 或高级 Provider 配置。

OMP bundled catalog 不进入正常管理列表。新建时禁止与 bundled Provider 或 bundled Provider/Model ID 发生不区分大小写的冲突。

已有冲突条目标记为“OMP 内置 Provider/模型覆盖”，整体只读、禁用测试并原样保留。

Bundled catalog 冲突检测使用随 OMP Switch 发布的 Provider ID 清单。清单由构建流程从对应 OMP 版本的官方 `pi-catalog` 生成，并按 `omp --version` 精确关联；不调用会混入用户认证、扩展或目标 `models.yml` 的 `omp models ls`。当前 OMP 没有匹配清单时，Provider 和模型管理整体只读，避免把无法分类的 built-in override 当作普通自定义配置；环境、配置查看、角色和设置仍可按各自规则使用。

实现状态（issue #7）：构建固定使用 `@oh-my-pi/pi-catalog@17.2.15`；`pnpm generate:bundled-manifest` 从其 `src/models.json` 生成按精确版本命名的只读资源，Rust 构建时校验并编译全部资源清单。运行时只按精确 OMP 版本查找该注册表，Provider 按 Custom Provider、Built-in Provider override、高级、不支持或清单缺失分类；清单缺失只冻结 Provider 与模型管理。

### 7.2 支持字段

```ts
type ProviderCreateForm = {
  provider: {
    id: string
    baseUrl: string
    defaultApi?: SupportedApi
    authMode: "api-key" | "none"
    apiKey?: string
  }
  firstModel: ModelForm
}

type SupportedApi =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages"
  | "google-generative-ai"
```

Provider 默认协议是模型的可选继承值，不是 Provider 的固定协议。

新增操作必须同时提交 Provider 和首个模型；表单通过全部 Provider、模型、协议和 ID 校验后，才在一次 `models.yml` 写入中创建完整节点。任何失败都不保存空 Provider。

实现状态（issue #8）：`create_custom_provider` 在 Rust 中一次接收 Provider 与首个 Model definition，并在写入前检查打开表单时的 `models.yml` 内容 Hash、当前配置和 bundled catalog。通过校验后才备份当前文件、在最新树中插入完整节点、fsync 临时文件、重新解析和验证未触及路径，再执行原子替换；任何失败均不写入空 Custom Provider。

实现状态（issue #9）：既有普通 Custom Provider 可编辑 Base URL、默认协议和认证方式；Provider ID 保持 Stable ID。Direct API Key 仅以 keep、replace、delete 明确意图进入一次保存，关闭或成功提交后清空表单输入。写入沿用内容 Hash、备份、临时重解析、未触及路径比较和原子替换。已有 `!command` 凭据保持高级/受限状态并只读，不会伪装成普通 Custom Provider。


### 7.3 Provider ID

新建规则：

- 去除首尾空白。
- 匹配 `[A-Za-z0-9][A-Za-z0-9._-]*`。
- 不允许 `/`、`:` 或空白。
- 在 `providers` 中按不区分大小写唯一。
- 不与 bundled Provider ID 发生不区分大小写冲突。

已有 Provider ID 不可修改。

### 7.4 Base URL

- 必填。
- 只接受 `http` 和 `https`。
- 本地服务允许 `http`。
- 保存时去除首尾空白和末尾 `/`。
- 不自动添加、删除或替换 `/v1`、`/v1beta` 等路径段。
- 根据模型有效协议显示最终测试地址预览。

协议地址：

- OpenAI Completions：`<baseUrl>/chat/completions`。
- OpenAI Responses：`<baseUrl>/responses`。
- Anthropic Messages：规范化后请求 `<baseUrl>/v1/messages`。
- Google Generative AI：`<baseUrl>/models/<modelId>:streamGenerateContent?alt=sse`；自定义 Base URL 必须自行包含 API 版本段。

### 7.5 API Key

MVP 只创建和使用直接文本 API Key：

- 新建时可以填写。
- 编辑时不返回已保存值，只返回 `hasApiKey`。
- 留空表示保留原值。
- 输入新值表示替换。
- 支持单独删除。
- 新值不得以 `!` 开头。
- 不加载 `.env`，不解析环境变量，不执行命令。
- 模型测试直接使用 Provider 保存的文本值。
- API Key 不进入应用设置、前端持久状态、后端概览缓存、IPC 响应、日志或测试结果。

已有 `!command` 不显示、不执行或编辑，并保持高级/受限分类和只读状态。

产品不额外显示 API Key 明文存储确认弹窗。

### 7.6 无认证

`authMode: "none"` 时不发送认证 Header。切换到无认证前，如果已有 API Key，确认是否删除；确认后删除。

### 7.7 高级 Provider

出现以下任一字段或行为时，Provider 整体只读并禁用模型测试：

- `headers`
- `compat`
- `discovery`
- `modelOverrides`
- `transport`
- `remoteCompaction`
- `disableStrictTools`
- `authHeader`
- `auth: oauth`
- override-only Provider
- bundled Provider/模型 ID 覆盖
- 其他不受支持的 Provider 字段

高级字段和值不返回前端，写回其他对象时原样保留。

高级、不支持和 Built-in Provider override 保持只读，不通过删除流程旁路；只允许普通可编辑 Custom Provider 进入删除检查。

## 8. 模型管理

### 8.1 支持字段

```ts
type ModelForm = {
  id: string
  name: string
  api?: SupportedApi
  reasoning: boolean
  input: Array<"text" | "image">
  contextWindow: number
  maxTokens: number
}
```

有效协议：

```text
Model.api ?? Provider.defaultApi
```

模型可以选择继承 Provider 默认协议，或显式选择四种支持协议。两者均为空时禁止保存和测试。

### 8.2 Model ID

新建规则：

- 去除首尾空白。
- 非空。
- 不含空白或控制字符。
- 同一 Provider 中按不区分大小写唯一。
- 不与同一 bundled Provider 下的 bundled Model ID 发生不区分大小写冲突。
- 允许 `/`、`.`、`_`、`-` 和普通 `:`。
- 禁止以 `:off`、`:minimal`、`:low`、`:medium`、`:high`、`:xhigh`、`:max`、`:auto` 结尾，避免与角色 Thinking 后缀歧义。

已有 Model ID 不可修改。

### 8.3 字段规则

- 名称必填。
- Text 和 Image 至少选择一种。
- `contextWindow` 必须是正整数。
- `maxTokens` 必须是正整数。
- `maxTokens` 不得大于 `contextWindow`，否则禁止保存。

### 8.4 不完整与只读模型

已有模型缺少产品必填字段时：

- 显示“配置不完整”。
- 不自动填值或写回。
- 用户补齐全部必填字段后才能保存、测试或分配新角色。
- 已有角色引用原样保留。

以下模型整体只读、不可测试或复制：

- 有效协议不受支持。
- 含模型级 `baseUrl` 覆盖。
- 含 `thinking`、`headers`、`compat`、`cost`、`supportsTools`、`premiumMultiplier`、`omitMaxOutputTokens`、`contextPromotionTarget`、`compactionModel`、`remoteCompaction` 或其他不受支持字段。
- 属于高级或 bundled override Provider。

只读模型不提供复制、删除或测试入口；issue #10 的 Model 管理验收采用此更保守规则。

### 8.5 复制与删除

复制模型：

- 只支持普通可编辑模型。
- 复制支持字段，不复制测试结果。
- 生成不冲突的临时 ID。
- 用户保存前不写入配置。

删除模型：

- 显示模型、Provider 和 `modelRoles` 引用。
- 自动清除简单 `modelRoles` 引用。
- 遍历完整 `config.yml` 检测其他或疑似选择器引用；发现时阻止删除并显示配置路径，不自动修改其他路径。
- 使用跨文件事务和备份。

实现状态（issue #10）：Provider 详情已通过 Rust application-service intent seam 与 React routed page seam 管理普通 Model definition 的列表、搜索、新增、编辑、复制和无引用删除。Model definition 按 `normal`、`incomplete`、`read-only` 分类；Stable ID 不可变，协议来源返回继承或模型覆盖，所有修改复用 models.yml 的 Hash、备份、临时重解析、未触及路径比较和原子替换。删除在提交前重新校验 config.yml 真实路径与 Hash，并识别大小写无关的精确、Thinking、数组和 Provider 通配符选择器；存在引用或删除最后模型时停止。未知字段仅保留在完整树中，不进入普通编辑表单。

## 9. 模型角色

### 9.1 角色范围

内置角色：

```text
default
smol
slow
vision
plan
designer
commit
tiny
task
advisor
```

MVP 同时支持自定义角色的创建、改名、模型选择、Thinking Level 和删除。

角色名：

- 首尾空白直接拒绝，不自动去除。
- 非空。
- 不含空白、`/`、`,` 或控制字符。
- 在 `modelRoles` 中精确唯一。

内置角色名称不可改名或删除，只能设置、清除其选择器。自定义角色可以创建、改名和删除；自定义角色没有独立于 `modelRoles` 键之外的“清除后保留”状态。自定义角色名按精确匹配唯一，不能与内置角色重名。

### 9.2 支持值

角色值只支持单值选择器：

```text
provider/model
provider/model:thinking
```
Simple role selector 的 Provider/Model 组成不得含空白、控制字符或逗号，且必须能按原 Provider/Model ID 无歧义还原。

例如：

```yaml
modelRoles:
  default: dnslin/gpt-5.6-luna:max
  advisor: dnslin/gpt-5.6-sol:max
  task: dnslin/gpt-5.6-terra:xhigh
```

支持的 Thinking Level：

```text
模型默认（不写后缀）
off
minimal
low
medium
high
xhigh
max
auto
```

不支持 `ultra`。OMP 会把未知 `:ultra` 当作 Model ID 的一部分。

### 9.3 高级角色配置

出现以下任一值时，整个角色页只读：

- `@role` 或其他别名。
- 逗号分隔候选。
- 数组形式。
- 无法解析的选择器。
- 未知 Thinking 后缀。

高级角色值原样保留，不自动降级或覆盖。

实现状态（issue #11）：角色页已通过 Rust AppService intent seam 与 React routed page seam 完整实现。十个内置角色支持设置/清除；自定义角色支持创建、改名、编辑、删除；只写入 `modelRoles` 中用户明确操作的键，并复用 Hash、备份、临时重解析、未触及路径比较和原子替换。无效简单引用与不支持协议引用可见且可修复；任一高级角色配置冻结整页并保留原值。Pencil 节点 i5xFP 导出为 `designs/50-roles-dirty.png`，真实 Tauri 脏状态截图保存在 `.artifacts/issue-11/roles-dirty-tauri-1536x961.png`，对比记录见 `.artifacts/issue-11/visual-comparison.txt`。

## 10. 引用、ID 和删除

已有 Provider ID 与 Model ID 不可重命名。更换 ID 的流程是：

```text
创建新对象
→ 手动调整引用
→ 确认旧对象无其他引用
→ 删除旧对象
```

删除 Provider 或模型时：

- 删除前扫描 `models.yml` 与 `config.yml` 的完整解析树，识别精确选择器、Thinking 后缀、Provider 通配符和选择器数组。
- 无引用时，只对 `models.yml` 中明确选中的完整节点执行单文件 Safe structured edit。
- 受支持 `modelRoles` 引用不会在本工单执行部分删除；界面明确交给同时修改两个文件的 Configuration transaction 流程。
- `modelRoles` 之外发现相关或疑似引用时阻止删除并显示安全路径摘要。
- 不自动修改 `retry.fallbackChains`、`task.agentModelOverrides` 或其他非受管路径。

实现状态（issue #13）：Rust application service 在最新完整树上完成 Model/Provider 引用扫描；普通无引用删除复用 Hash、当前备份、临时文件重解析、未触及路径比较和原子替换。Provider 删除先合并检查其全部模型引用；非受管引用阻止写入，受支持角色引用明确停在跨文件事务入口。React 确认 Dialog 展示删除对象、包含模型、角色路径、其他引用和备份行为；确认按钮在阻止状态禁用。

## 11. 模型连接测试

### 11.1 入口和并发

- Provider 详情模型列表。
- 已保存模型的编辑表单。
- 概览页。
- 不支持未保存模型测试。
- 全应用同时只允许一个测试请求。
- 不自动测试；只由用户手动发起。

第一次模型测试前显示一次非阻塞说明：测试会向所选 Provider 发起真实 API 请求，可能产生费用。用户确认后记录本地偏好，不再重复提示；更换应用数据目录不影响配置文件。

该偏好属于轻量应用设置，不包含请求、响应或 OMP 配置数据。

### 11.2 请求

Rust 根据已保存配置构造四种协议的固定最小请求：

- 使用有效协议和最终地址。
- 使用直接文本 API Key，或无认证模式。
- 使用固定无敏感信息提示词。
- 请求输出上限使用协议允许的安全最小值。
- 前端不提交或接收 API Key。
- 存在自定义 Header、高级 Provider、高级模型、命令凭据或不支持协议时禁用测试。

### 11.3 结果

显示：

- 成功或失败。
- Provider、模型和有效协议。
- 请求耗时。
- HTTP 状态码。
- 脱敏错误分类和处理建议。

支持超时和主动取消。不保存完整请求体、完整响应正文或测试历史；只在当前应用运行期间保留最近一次脱敏结果。
- 概览刷新会重新同步后端测试状态；Target configuration 或 `models.yml` Hash 变化会清除旧结果，未变化则保留最近结果。

需要区分 Base URL、DNS、连接、TLS、超时、取消、401、403、404、429、5xx 和响应格式错误。

## 12. 备份

备份保存到跨平台应用数据目录：

```text
<app-data>/backups/<agent-dir-fingerprint>/
  models/
  config/
  transactions/
```

规则：

- 每个配置目录独立命名空间。
- `models.yml` 和 `config.yml` 各保留最近 10 份。
- 同一跨文件事务共享事务 ID。
- 当前事务备份失败时中止写入。
- 清理旧备份失败只记录警告，不回滚已安全完成的写入。
- 提供“打开备份目录”，不提供备份列表或恢复 UI。

## 13. 页面和交互

页面：

```text
首次检查
概览
Providers
Provider 详情
角色
设置
```

通用要求：

- Loading、Empty、Error、Normal 状态完整。
- 使用 Skeleton，不使用整页旋转加载。
- 表单由 React Hook Form 管理，规则由 Zod 管理。
- 未修改或校验失败时禁用保存。
- 保存期间禁止重复提交。
- 未保存修改离开时确认。
- 删除使用确认对话框。
- 成功和失败摘要使用 Sonner；详细错误放在页面或弹窗。
- 支持键盘操作和减少动画偏好。
- Motion 只用于轻量状态变化。

`designs/omp-switch.pen` 是 MVP 已批准的视觉实现契约。页面布局、窗口尺寸基准、导航、组件形态、层级、文案、字体、字号、字重、颜色、间距、圆角、边框、阴影、图标、表格、Dialog/Sheet 尺寸和状态表现必须 1:1 还原。不得以 shadcn/ui 或平台默认样式替代设计；组件库只作为实现基础。

上述 1:1 规则适用 issue #5 已确认的 Setup 最外层卡片例外；该例外不授权修改共享 token、检测表格、状态行、按钮或其他页面卡片。

## 14. 技术栈

| 用途 | 技术 |
|---|---|
| 桌面框架 | Tauri 2 |
| 后端 | Rust |
| 前端 | React + TypeScript Strict Mode |
| 构建 | Vite |
| UI | shadcn/ui |
| 样式 | Tailwind CSS |
| 界面状态 | Zustand，仅轻量界面状态 |
| 表单 | React Hook Form + Zod + `@hookform/resolvers` |
| 图标 | lucide-react |
| 动画 | Motion (`motion/react`) |
| 通知 | Sonner |
| 前端测试 | Vitest + Testing Library |
| Rust 测试 | Cargo test |
| 包管理 | pnpm |

MVP 不引入数据库、ORM、嵌入式终端、多窗口架构或 OMP Sidecar。

## 15. 测试与发布阻断项

### 15.1 Rust 测试

必须覆盖：

- OMP 可执行文件、版本和配置路径命令处理。
- `.yml` / `.yaml` / JSON 文件发现状态。
- 符号链接和重解析点安全处理。
- Provider 与模型 YAML 解析。
- case-insensitive ID 冲突。
- OMP 版本对应的 bundled Provider 清单存在性和冲突检测。
- 有效协议继承与四种协议 URL、请求和响应。
- 直接 API Key 脱敏。
- 高级 Provider、模型和角色只读分类。
- 未触及路径逐值保留。
- 首次模型测试费用说明。
- 内置角色只能设置或清除，自定义角色执行 CRUD。
- Hash 冲突检测。
- 单文件原子写入。
- 跨文件事务、崩溃恢复和现场副本。
- 每文件每配置目录备份保留。
- 引用扫描和阻止删除。
- 网络错误分类、超时和取消。

### 15.2 前端测试

必须覆盖：

- Provider、模型和角色表单校验。
- ID 不可修改和 case-insensitive 判重。
- Provider 默认协议和模型覆盖。
- API Key 保留、替换和删除。
- 高级对象只读状态。
- 不支持协议状态。
- 模型字段不完整状态。
- 自定义角色 CRUD。
- 高级角色导致全页只读。
- 删除引用提示和阻止状态。
- 配置冲突、写入失败和事务恢复提示。
- 模型测试单并发、取消和结果状态。
- Loading、Empty、Error、Normal 状态。

### 15.3 三平台手工验收

每个正式支持平台必须验证：

- 从 PATH 和手动路径检测 OMP。
- `omp config path` 成功与失败。
- 配置目录权限和链接场景。
- 创建缺失目录和 `.yml`。
- 读取脱敏现有配置。
- Provider、模型、角色完整流程。
- 四种协议的成功和失败测试。
- 外部修改 Hash 冲突。
- 模拟单文件写入失败。
- 模拟跨文件事务中断和启动恢复。
- 备份保留与目录隔离。
- 日志和 IPC 不出现 API Key。
- 在 1536×1024 设计基准视口对 `designs/omp-switch.pen` 中每个页面画板执行实现截图与 Pencil 导出截图对比；无未经批准的布局、尺寸、间距、字体、颜色、圆角、边框、阴影、图标、文案或状态表现偏差。

## 16. MVP 验收标准

发布前必须全部满足：

### 环境

- [ ] 支持矩阵内三个平台全部通过真实运行验收。
- [ ] 可以自动检测和手动选择 OMP。
- [ ] 可以显示 OMP 版本。
- [ ] 可以通过 `omp config path` 获取权威配置目录。
- [ ] 路径命令失败时不猜测目录。
- [ ] `.yaml` 或旧 JSON 状态不会被误写。

### Provider 和模型

- [ ] 可以原子创建自定义 Provider 与首个模型，并编辑、搜索和删除已有普通自定义 Provider。
- [ ] 可以创建、编辑、复制、搜索和删除普通模型。
- [ ] 已有 ID 不可修改。
- [ ] ID 按不区分大小写判重。
- [ ] 新建 ID 不覆盖 bundled Provider 或模型。
- [ ] Provider 默认协议和模型协议覆盖正确。
- [ ] 四种支持协议均可测试。
- [ ] 高级、不支持或不完整对象不会被误编辑或误测试。

### 角色

- [ ] 可以管理十个内置角色。
- [ ] 可以创建、改名、编辑和删除自定义角色。
- [ ] 角色值只写单值模型选择器。
- [ ] Thinking Level 不写入 `ultra`。
- [ ] 高级角色配置不会被静默覆盖。

### 配置安全

- [ ] 只修改 `providers` 和 `modelRoles` 中的目标路径。
- [ ] 未触及路径的值逐值不变。
- [ ] 写入失败不损坏原文件。
- [ ] 外部修改不会被静默覆盖。
- [ ] 当前事务备份失败时不写入。
- [ ] 跨文件中断不会留下部分提交。
- [ ] 其他配置路径存在引用时阻止删除。
- [ ] API Key 不进入 UI 返回、应用设置、IPC、日志或测试结果。

### 视觉还原

- [ ] `designs/omp-switch.pen` 中的 Foundations 和 Components 已实现为唯一设计 token 与共享组件来源。
- [ ] Setup 按 issue #5 经确认的无外层卡片基准还原，Overview、Providers List、Provider Detail、Roles Dirty、Settings 画板在 1536×1024 基准视口 1:1 还原。
- [ ] 原型中定义的 Provider 创建步骤与 Model 创建 Sheet 按对应设计 1:1 还原。
- [ ] 每个已实现画板都有实现截图和 Pencil 导出截图的视觉对比证据；可见偏差在关闭工单前修复。
- [ ] 除 issue #5 已记录并由产品负责人确认的 Setup 最外层卡片例外外，仅允许操作系统原生窗口控件、文件选择器、路径格式和快捷键标签产生平台差异；其他视觉差异必须先更新并重新批准 `.pen` 文件。

### 范围

- [ ] 不包含启动 OMP。
- [ ] 不包含终端、会话或工作目录。
- [ ] 不管理项目配置或 Profile 生命周期。
- [ ] 不编辑 OAuth、命令凭据、自定义 Header 或高级 OMP 配置。
- [ ] 不承诺 YAML 格式与注释保真。
