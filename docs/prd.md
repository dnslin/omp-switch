# OMP Switch PRD

> 产品版本：v0.1  
> 文档状态：已确认，待实现  
> 产品形态：跨平台桌面应用

## 1. 文档职责

本 PRD 基于 `docs/mvp.md`，负责定义 MVP 范围内的产品行为、业务规则、状态和错误语义。

- 范围和发布阻断项以 `mvp.md` 为准。
- 本文可以细化，但不得扩大或缩小 MVP。
- `flow.md` 基于 MVP 和 PRD 描述具体交互，不得改变本文规则。
- 三份文档冲突时：范围问题回到 MVP；产品规则以 PRD 为准并修正 Flow。
- `designs/omp-switch.pen` 是已批准的视觉权威。实现必须按其中 Foundations、Components 和页面画板 1:1 还原。

## 2. 产品概述

OMP Switch 是 OMP 配置的安全结构化编辑器，帮助用户通过桌面界面管理：

- 自定义 Provider。
- Provider 下的模型。
- OMP 模型角色。
- 模型连接测试。
- 配置备份、冲突检测和安全写入。
- OMP 可执行文件及权威配置目录状态。

产品不替代 OMP，不启动 OMP，也不管理终端、会话、项目工作目录或项目级 `.omp/config.yml`。

### 2.1 核心目标

1. 降低手工编辑 YAML 的错误率。
2. 不静默修改用户未操作的配置。
3. 不泄露 API Key。
4. 不因写入失败、进程崩溃或外部修改损坏配置。
5. 准确反映 OMP 的 Provider、模型协议和角色选择器语义。
6. 在 macOS、Windows 和 Ubuntu 的正式支持矩阵上提供一致行为。

### 2.2 非目标

- 不成为完整 OMP 设置编辑器。
- 不提供 OMP bundled Provider/模型管理。
- 不支持所有 OMP 协议和高级字段。
- 不修复、清理或规范化产品不理解的配置。
- 不保证 YAML 注释和格式保真。

## 3. 支持平台

正式 MVP 必须支持：

- macOS 13 Ventura 及以上：Intel、Apple Silicon。
- Windows 10 22H2、Windows 11：x64。
- Ubuntu 22.04 LTS、Ubuntu 24.04 LTS：x64。

界面采用平台中立的桌面设计。除系统原生窗口控件、文件选择、目录打开、路径格式和快捷键标签外，三个平台的界面必须与 `designs/omp-switch.pen` 在 1536×1024 基准视口 1:1 一致。

Issue #5 经产品负责人确认后修订 Setup 视觉基准：Setup 保留 `.pen` 的 Foundations、Components、检测表格、状态行、间距和操作区，但不绘制最外层整页卡片的背景、边框、圆角、阴影和内边距；以 `.artifacts/issue-5/implementation-1536x1024.png` 与 `.artifacts/issue-5/responsive-cardless-1100x720.png` 作为该页面的批准基准。此例外不适用于其他页面或 Setup 内部组件。

## 4. 产品结构

页面：

```text
首次检查
概览
Providers
Provider 详情
角色
设置
```

侧边栏：

```text
概览
Providers
角色
设置
```

Provider 详情包含模型管理。模型不设置独立顶层页面。

## 5. 领域对象

### 5.1 Target configuration

Target configuration 是所选 OMP 可执行文件执行：

```text
omp config path
```

后返回的全局 Agent 配置目录。返回值是权威路径；产品不猜测路径，也不管理项目级配置。

### 5.2 Custom Provider

可编辑 Custom Provider 是 `models.yml.providers` 中满足以下条件的条目：

- 拥有非空 `models` 列表。
- Provider ID 及 Provider/Model ID 不与当前 OMP bundled catalog 发生不区分大小写的冲突。
- 只使用 MVP 支持的 Provider 字段、模型字段、协议和凭据形式。

### 5.3 Built-in Provider override

如果 Provider ID 或 Provider/Model ID 与 OMP bundled catalog 相同，OMP 会将配置合并到 bundled Provider/模型并改变其传输行为。此类现有条目：

- 显示为“OMP 内置 Provider/模型覆盖”。
- 整体只读。
- 禁用模型测试。
- 原样保留。

新建时禁止产生这种不区分大小写的冲突。

### 5.4 Stable ID

已有 Provider ID 和 Model ID 不可修改。

- Provider ID 在全部 `providers` 中按不区分大小写唯一。
- Model ID 在同一 Provider 中按不区分大小写唯一。
- 更换 ID 需要创建新对象、调整引用并删除旧对象。

### 5.5 Model role

角色值只支持：

```text
provider/model
provider/model:thinking
```

内置角色和自定义角色都使用相同结构。不支持别名、数组、多候选等属于高级角色配置。

## 6. OMP 环境检测

### 6.1 检测顺序

1. 用户保存的 OMP 可执行文件路径。
2. 系统 `PATH` 中的 `omp`。
3. 用户手动选择的文件。

### 6.2 固定命令

```text
omp --version
omp config path
```

OMP Switch 静默执行命令。`omp config path` 由 OMP 自身实现，会初始化 Settings、访问 `agent.db`，并可能在没有现有主 YAML 时执行 OMP 自身的旧设置迁移。产品文档和错误详情必须如实记录这一事实，但不额外弹出确认。

### 6.3 成功条件

- 可执行文件存在并可运行。
- `omp --version` 成功。
- `omp config path` 成功且输出一个可解析绝对目录。
- 目标目录或其父目录可按产品规则访问。

### 6.4 失败行为

- 不回退到 `~/.omp/agent`。
- 不让用户绕过命令任意选择配置目录。
- 显示发生了什么、可能原因和处理方式。
- 技术详情可包含脱敏 stderr 和退出码。
- 提供重新检测和重新选择 OMP。

成功状态下点击“重新检测”时，当前 OMP 信息保持在背景中，不回退到初始检测页面。产品使用窗口级半透明模糊遮罩，在窗口中心单独显示 Dot Matrix 加载面板与“正在重新检测 OMP”文案，最短展示 1200ms；遮罩期间禁用重新检测和进入应用，完成后一次性更新结果。

OMP 版本只用于诊断和兼容报告，不作为单独的写入许可。每次写入必须通过当前目标目录、文件结构、未触及路径、临时文件重解析和引用完整性检查；任一检查失败时进入只读或停止保存。

### 6.5 OMP 路径切换

用户选择新的 OMP 后：

```text
验证新可执行文件
→ 获取版本
→ 获取新权威配置目录
→ 显示目录变化
→ 用户确认切换
→ 保存 OMP 路径
→ 重新读取配置
```

### 6.6 Bundled Provider 清单

创建或编辑普通 Custom Provider 前，Rust 必须加载与 `omp --version` 精确对应的 bundled Provider ID 清单。

清单由 OMP Switch 构建流程从对应版本的官方 `pi-catalog` 生成并作为只读资源发布。产品不使用 `omp models ls` 生成该清单，因为该命令只列出当前认证后可用模型，还可能混入扩展、缓存和用户配置，不能证明完整的 bundled catalog。

当前 OMP 版本没有匹配清单时，Provider 和模型管理整体只读；环境检测、配置查看、角色和设置仍可按各自规则使用。该限制是 bundled override 分类能力缺失导致的安全门槛，不代表按 SemVer 推断整个配置格式不兼容。

实现状态（issue #7）：`@oh-my-pi/pi-catalog@17.2.15` 被锁定为构建输入。`pnpm generate:bundled-manifest` 只接受该官方包及其声明版本，提取 Provider/model ID；`build.rs` 验证每个资源文件名与内容版本一致后编译完整注册表。运行时不执行 `omp models ls`，仅精确匹配清单；无匹配时环境、Target configuration 查看、Model role 和设置继续按自己的安全规则运行。

新 OMP 检测失败时保留当前可用 OMP 路径。

实现约束（issue #4）：手动选择先调用无副作用验证意图；只有用户在验证结果中确认后才提交新的 OMP 路径。检测 DTO 同时返回目标访问性及规范 `.yml` 文件的正常、缺失或只读状态；缺失文件不视为可进入主界面的完整设置。

## 7. 配置文件发现

### 7.1 支持文件

可写文件仅为：

```text
models.yml
config.yml
```

### 7.2 `.yaml`

- 只有 `.yaml` 时，配置进入只读状态并提示 MVP 只写入 `.yml`。
- `.yml` 和 `.yaml` 同时存在时使用 `.yml`，并显示 `.yaml` 不会被修改。
- 不自动重命名、删除或迁移 `.yaml`。

### 7.3 旧 JSON

检测到旧 JSON 且缺少受支持 YAML 时：

- 不创建空 YAML 抢占读取优先级。
- 不在 OMP Switch 中迁移或编辑 JSON。
- 要求用户先使用 OMP 完成官方迁移。

### 7.4 文件或目录缺失

当权威配置目录、`models.yml` 或 `config.yml` 缺失时：

- 显示完整目标路径。
- 列出将创建的目录和文件。
- 用户确认后创建最小有效结构。
- 不覆盖已有文件。
- 创建完成后重新检测和读取。

最小有效文件内容：

```yaml
# models.yml
providers: {}
```

```yaml
# config.yml
modelRoles: {}
```

使用 UTF-8 和 LF；创建后立即重新解析。任何创建或验证失败都不得留下部分文件。

### 7.5 符号链接和重解析点

- 解析并显示真实目标。
- 备份真实目标内容。
- 临时文件必须位于真实目标所在目录。
- 目标变化、链接循环、无法确认目标或权限异常时拒绝写入。
- 产品不创建链接。

实现状态（issue #5）：应用服务以 `omp config path` 结果执行完整发现，识别规范 `.yml`、同名 `.yaml`、`models.json`、`settings.json` / `config.json`、缺失路径、YAML 错误及链接或重解析点，并返回无业务配置内容的状态 DTO。初始化只接受已验证 OMP 可执行文件意图与用户已确认的创建清单，不接受前端指定目标目录；写入前重新验证创建清单和最近现有父路径的真实目标。创建失败会回滚本次产生的临时文件、规范文件和目录；回滚不完整时明确报告残留路径，成功后重新解析并重新发现。
实现状态（issue #6）：概览读取由 Rust application service 的 `get_overview_load` 完成，先重新验证 `omp config path`，再读取真实目标文件、计算原始 Hash、保留完整 `serde_yaml::Value` 树并生成无密钥 DTO。React 只消费 `get_overview_load` IPC 返回的 `hasApiKey` 等安全元数据；读取失败先清空业务快照，成功后一次性刷新统计。概览页按 `03 Page / Overview` 的尺寸、密度、组件和响应式规则实现，Windows/其他平台均保留 Tauri 原生标题栏。Provider/Model Select 恢复并保存轻量 pair，切换 Provider 时清理不兼容 Model，摘要随 pair 更新；失效选择只提示一次并串行保存修正后的完整 UI settings。只读或不完整安全投影仍允许查看，模型测试按钮保持禁用。


## 8. 配置读取和安全写入

### 8.1 解析要求

Rust 负责：

- 读取完整 YAML 数据树。
- 解析 Provider、模型和角色的支持结构。
- 分类高级、不支持和不完整对象。
- 保存内容 Hash。
- 保留未知路径和值。
- 返回无敏感值的前端 DTO。

### 8.2 修改范围

`models.yml`：

- 只能定点修改 `providers` 下目标 Provider、模型或支持字段。
- 不整体替换 `providers`。
- 不修改根节点其他路径。
- 不修改其他 Provider。
- 不修改目标对象中的未知字段。

`config.yml`：

- 只能定点修改 `modelRoles` 中目标角色键。
- 不整体替换 `modelRoles`。
- 不修改其他设置路径。

### 8.3 未触及路径验证

写回临时文件并重新解析后，比较所有未触及路径：

- 路径必须仍然存在。
- 值必须深度相等。
- 任何差异都中止替换。

明确删除完整 Provider、模型或角色时，对应目标路径不属于未触及路径。

### 8.4 YAML 保真边界

保证：

- 数据语义。
- 未触及路径和值。
- OMP 可重新解析。

不保证：

- 注释。
- 空行。
- 缩进。
- 标量引号样式。
- 原始键顺序。

### 8.5 单文件写入

```text
锁定目标
→ 重新读取
→ 比较 Hash
→ 创建备份
→ 定点修改最新树
→ 验证业务规则
→ 写入同目录临时文件
→ fsync
→ 重新解析
→ 验证未触及路径
→ 原子替换
→ 重新读取
```

备份失败、临时文件验证失败、Hash 冲突或替换失败时，原文件不变。

### 8.6 跨文件事务

需要同时修改 `models.yml` 和 `config.yml` 的操作使用共享事务：

- 两个文件都在写入前锁定、重读、Hash 比较和备份。
- 所有临时文件通过验证后，写入持久事务清单。
- 清单记录事务 ID、逻辑 Target、目标、显式备份分区、备份、原始 Hash 和最终 Hash；真实目标路径按当前逻辑入口重新解析并精确核对。
- 文件依次原子替换。
全部成功后清理事务清单；若清理失败，保留清单并返回明确错误，后续启动按最终 Hash 完成提交清理。

启动恢复规则是确定的：

- 所有目标均匹配最终 Hash：只完成提交清理，不要求再次读取共享备份。
- 否则先保存当前现场副本，再预检同一事务的全部备份并恢复所有文件。
- 不允许部分恢复。
文件符号链接仍指向原真实目标时，恢复使用清单记录的原真实 Target 和文件路径；Target 目录符号链接发生重定向时，启动仍按逻辑 Target 找到原事务并锁定原真实 Target 后恢复，不会静默跳过清单。
删除确认发生 `models.yml` 或 `config.yml` Hash 冲突时，确认 Dialog 保留删除对象和影响清单；删除按钮禁用，用户只能选择重新读取或取消，禁止盲目重试。

### 8.7 外部修改

MVP 不实时监听文件。

保存前重新读取并比较 Hash。发生变化时：

- 本次保存停止。
- 保留表单。
- 提示重新加载会丢失未保存修改。
- 提供取消和重新加载。
- 不自动合并。

## 9. Provider 页面与规则

### 9.1 Providers 列表

显示：

- Provider ID。
- Base URL。
- Provider 默认协议或“由模型指定”。
- API Key 状态：已配置、未配置、无需认证、不支持凭据。
- 模型数量。
- 状态：正常、配置不完整、只读、高级配置、bundled override。

支持搜索 Provider ID、Base URL 和协议。

### 9.2 新增 Provider 和首个模型

MVP 不保存空 Provider。新增表单一次收集 Provider 和首个模型：

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
```

支持协议：

```text
openai-completions
openai-responses
anthropic-messages
google-generative-ai
```

Provider 默认协议可留空，但首个模型必须最终能够通过 `Model.api ?? Provider.defaultApi` 得到有效协议。保存时一次性新增 Provider 节点和首个模型；任何校验、冲突、备份或写入失败都不留下空 Provider。

实现状态（issue #8）：Provider 创建表单经两步收集上述字段。提交成功后刷新 Providers 数据并进入新 Provider 详情；字段校验、Hash 冲突和普通写入失败都保留表单。Hash 冲突只提供显式“重新读取”，不会自动合并或覆盖外部修改。


### 9.3 Provider ID

- 去除首尾空白。
- 匹配 `[A-Za-z0-9][A-Za-z0-9._-]*`。
- 不含 `/`、`:` 或空白。
- 按不区分大小写唯一。
- 不与 bundled Provider ID 冲突。
- 创建后只读。

### 9.4 Base URL

- 必填，只支持 `http` 和 `https`。
- 去除首尾空白和末尾 `/`。
- 不自动补 `/v1` 或 `/v1beta`。
- 保留用户提供的路径前缀。
- 表单根据模型有效协议显示最终请求地址预览。

地址规则：

| 协议 | 最终测试地址 |
|---|---|
| OpenAI Completions | `<baseUrl>/chat/completions` |
| OpenAI Responses | `<baseUrl>/responses` |
| Anthropic Messages | 规范化为 `<baseUrl>/v1/messages` |
| Google Generative AI | `<baseUrl>/models/<modelId>:streamGenerateContent?alt=sse` |

Google 自定义 Base URL 必须由用户包含所需 API 版本段。

### 9.5 API Key

Rust 返回：

```ts
type ProviderView = {
  id: string
  baseUrl: string
  defaultApi?: SupportedApi
  authMode: "api-key" | "none" | "unsupported"
  hasApiKey: boolean
  modelCount: number
  editable: boolean
  readOnlyReason?: string
}
```

不返回 API Key 值。

编辑规则：

- 输入框默认空。
- 留空保留。
- 新值替换。
- 单独操作删除。
- 新值不得以 `!` 开头。
- 切换无认证时确认删除已有 Key。
- 表单关闭和提交后清空前端输入。

已有 `!command`：

- 不回传、执行或编辑。
- 保持高级/受限分类和只读状态，不得伪装成普通 Custom Provider。
- 禁用测试。

### 9.6 高级 Provider

出现自定义 Header、OAuth、compat、discovery、modelOverrides、transport、remoteCompaction、disableStrictTools、authHeader、override-only 或 bundled override 等高级配置时：

- Provider 整体只读。
- 禁止模型测试。
- 高级字段值不返回前端。
- 写入其他对象时原样保留。

高级、不支持和 Built-in Provider override 保持只读，不通过删除流程旁路；只有普通可编辑 Custom Provider 可以进入删除引用检查。

## 10. 模型页面与规则

### 10.1 模型列表

使用适合桌面应用的表格，显示：

- 名称。
- Model ID。
- 完整选择器 `provider/model`。
- 有效协议及来源：模型指定或继承 Provider。
- 输入能力。
- Reasoning。
- Context Window。
- Max Tokens。
- `modelRoles` 引用数量。
- 状态和操作。

### 10.2 模型表单

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

协议字段选项：

- 继承 Provider。
- 四种支持协议。

### 10.3 Model ID

- 去除首尾空白。
- 非空，不含空白或控制字符。
- 同一 Provider 内按不区分大小写唯一。
- 不与同一 bundled Provider 的 bundled Model ID 冲突。
- 允许 `/`、`.`、`_`、`-` 和普通 `:`。
- 不允许以 `:off`、`:minimal`、`:low`、`:medium`、`:high`、`:xhigh`、`:max`、`:auto` 结尾。
- 创建后只读。

### 10.4 能力和 Token

- Text 和 Image 至少一种。
- Context Window 为正整数。
- Max Tokens 为正整数。
- Max Tokens 不得大于 Context Window；违反时禁止保存。

### 10.5 不完整模型

已有模型缺少名称、输入能力、Context Window 或 Max Tokens 时：

- 显示“配置不完整”。
- 不自动补齐。
- 可以打开表单补齐。
- 补齐前不可测试或分配新角色。
- 已有角色引用保留。

### 10.6 只读模型

以下模型只读、不可测试和复制：

- 不支持协议。
- 模型级 Base URL 覆盖。
- 含 `thinking`、`headers`、`compat`、`cost`、`supportsTools`、`premiumMultiplier`、`omitMaxOutputTokens`、`contextPromotionTarget`、`compactionModel`、`remoteCompaction` 或未知字段。
- 所属 Provider 只读。

只读模型不提供复制、删除或测试入口；issue #10 的 Model 管理验收采用此更保守规则。

### 10.7 复制模型

- 仅普通可编辑模型可复制。
- 复制表单支持字段。
- 临时 ID 按不区分大小写确保不冲突。
- 保存前不修改配置。

实现状态（issue #10）：Model 列表 DTO 额外返回 `status`、`referenceCount` 和安全引用路径；不完整对象仍可进入修复表单，但缺失字段保持空值，补齐前不能保存或测试。普通模型复制只预填支持字段并要求新的唯一 ID；只读模型的复制、删除和测试入口均锁定。删除确认展示引用和最后模型阻止原因，Rust 在 models.yml 原子替换前再次比较 config.yml Hash，外部新增引用时拒绝提交。

## 11. 角色页面与规则

### 11.1 角色

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

用户可以设置或清除十个内置角色；内置角色不能改名或删除。用户可以创建、改名、编辑和删除自定义角色。

### 11.2 角色值

只支持：

```text
provider/model
provider/model:thinking
```
Simple selector 的 Provider/Model ID 必须不含空白、控制字符或逗号，并能按原 ID 无歧义 round-trip；不满足时不提供角色分配入口。

例如：

```yaml
modelRoles:
  default: dnslin/gpt-5.6-luna:max
  advisor: dnslin/gpt-5.6-sol:max
  task: dnslin/gpt-5.6-terra:xhigh
```

Thinking Level：

```text
模型默认
off
minimal
low
medium
high
xhigh
max
auto
```

“模型默认”不写后缀。不提供 `ultra`。

### 11.3 自定义角色

自定义角色支持：

- 创建。
- 改名。
- 选择模型。
- 设置 Thinking Level。
- 删除。

角色名称必须非空且不含空白、`/`、`,` 或控制字符。

自定义角色名按精确匹配唯一，不能与内置角色重名。改名在同一次 `config.yml` 定点写入中删除旧键并新增新键；发生 Hash 冲突时不保存任何部分。

自定义角色只存在于 `modelRoles` 键中，没有“清除选择器但保留空角色”的状态。删除该键即删除自定义角色。

### 11.4 高级角色配置

任一 `modelRoles` 值为以下形式时，整个角色页只读：

- `@role` 或其他别名。
- 逗号候选。
- 数组。
- 未知 Thinking 后缀。
- 无法解析的选择器。

页面显示导致只读的角色和原因；保存其他页面时原样保留所有角色值。

### 11.5 角色状态

- 正常。
- 未配置。
- Provider 不存在。
- 模型不存在。
- 模型配置不完整。
- 不支持协议。
- 高级配置。

不自动清除已有无效角色。用户明确重选、清除或删除后才修改。

实现状态（issue #11）：React 页面与 Rust 应用服务已按上述边界实现；Pencil 节点 i5xFP 导出为 `designs/50-roles-dirty.png`，真实 Tauri 脏状态截图保存在 `.artifacts/issue-11/roles-dirty-tauri-1536x961.png`，对比记录见 `.artifacts/issue-11/visual-comparison.txt`。

## 12. 删除与引用完整性

### 12.1 删除模型

确认内容：

- Provider 和 Model ID。
- 受影响的受支持 `modelRoles`，并明确将清除哪些角色键。
- 将删除完整模型节点。
- 发生角色引用时，明确 `models.yml` 与 `config.yml` 都会使用同一事务 ID 备份并修改。
- 备份说明。

执行前：

1. 扫描两份完整配置树中的所有值，包括非字符串 YAML key 下的嵌套值；路径摘要对非字符串 key 做安全序列化并脱敏。
2. 识别精确选择器、Thinking 后缀、`provider/*` 和选择器数组。
3. 先按所属 Provider 的现存完整 Model ID 精确解析选择器；仅当完整 ID 不存在时才剥离受支持 Thinking 后缀。未知后缀若命中目标基础 ID，归入疑似非受管引用并阻止删除；现存完整 ID（例如 `second:ultra`）优先于后缀解释。
4. 无引用时，复用单文件 Safe structured edit 删除 `models.yml` 中明确选中的节点。
5. 只有受支持 `modelRoles` 引用时，生成并验证两个临时文件，再持久化记录事务 ID、真实目标、共享备份和两组原始/最终 Hash，依次原子替换；不执行 models-only 部分删除。
6. `modelRoles` 之外存在相关或疑似引用时阻止删除，并显示安全路径摘要；阻止状态说明不会写入配置或创建备份。
### 12.2 删除 Provider

确认内容：

- Provider ID。
- 包含的全部模型。
- 所有 `modelRoles` 引用。
- 是否存在其他路径引用。
- 删除会移除 Provider 及全部模型。
- 备份说明。

执行规则与删除模型相同；Provider 必须先检查其全部模型选择器，不能通过删除 Provider 绕过引用完整性检查。无引用删除只修改 `models.yml`；支持角色引用时转入跨文件事务，其他路径引用阻止删除。
Provider 整体删除还要求其每个 Model definition 都是可编辑、非高级对象；不能因 Provider 顶层字段普通而通过删除 Provider 绕过只读模型边界。

实现状态（issue #14）：Rust application service 已实现受支持 Model role 引用的双文件锁定、锁内重读与 Hash 复核、同一事务 ID 的共享备份、全部临时文件解析验证、持久事务清单、依次原子替换和启动确定性恢复。完整提交只清理清单；其他状态先保存现场并预检全部备份，再整体恢复 `models.yml` 与 `config.yml`，恢复失败时 Target configuration 进入只读并展示安全路径。
实现状态补充（issue #14）：Model 与 Provider 删除确认共用批准 Dialog，列出将删除对象、将清除的角色键、非受管引用和双文件备份/修改范围；Rust 故障点测试覆盖共享备份后、清单后、第一/第二文件替换后及清理时中断。

### 12.3 不自动修改的引用

OMP Switch 不自动修改：

- `retry.fallbackChains`。
- `task.agentModelOverrides`。
- 其他 `config.yml` 路径。

检测到引用时阻止删除，由用户在 OMP 或外部编辑器中先处理。

## 13. 模型连接测试

### 13.1 测试限制

- 只测试已保存模型。
- 用户手动发起。
- 全应用单并发。
- 支持取消和超时。
- 高级、只读、不完整或不支持协议的对象不能测试。

第一次发起模型测试前显示一次非阻塞费用说明。用户确认后在应用设置中保存 `modelTestCostNoticeAccepted: true`；测试仍只能由用户手动触发。

费用说明偏好不包含 Provider、模型、请求或响应数据。

### 13.2 请求构造

Rust 读取已保存 Provider 和模型，计算有效协议并构造固定最小请求。

- OpenAI 协议使用 Bearer Token。
- Anthropic 官方地址使用 `X-Api-Key` 与 `anthropic-version: 2023-06-01`；自定义兼容地址按 OMP 当前 Anthropic Provider 行为使用认证 Header。
- Google 使用 `x-goog-api-key`。
- 无认证时不添加认证 Header。
- 前端不构造请求、不提交 API Key。
- 不发送自定义 Header，因为此类 Provider 已禁用测试。

### 13.3 结果 DTO

```ts
type ModelTestResult = {
  success: boolean
  providerId: string
  modelId: string
  protocol: SupportedApi
  latencyMs: number
  status?: number
  message: string
  errorCode?: string
}
```

不返回请求 Header、API Key、完整请求或完整响应。
- 概览或 Provider 刷新后重新同步测试状态；如果绑定的 Target configuration 或 `models.yml` Hash 已变化，旧结果立即失效。

### 13.4 错误分类

| 错误 | 提示方向 |
|---|---|
| Base URL 无效 | 检查地址 |
| DNS 失败 | 检查域名或网络 |
| 连接失败 | 检查服务状态 |
| TLS 错误 | 检查证书和 HTTPS |
| 超时 | 检查网络或超时设置 |
| 401 | 检查 API Key |
| 403 | 检查权限 |
| 404 | 检查最终地址和 Model ID |
| 429 | 频率或额度限制 |
| 5xx | Provider 服务异常 |
| 响应格式错误 | 检查协议兼容性 |
| 用户取消 | 测试已取消，不作为错误 |

## 14. 备份

备份根目录位于应用数据目录：

```text
<app-data>/backups/<agent-dir-fingerprint>/
  models/
  config/
  transactions/
```

- 每个 agentDir 独立。
- 每个目标文件独立保留最近 10 份。
- 事务备份共享事务 ID。
- 当前备份失败时中止写入。
- 清理旧备份失败只显示或记录警告。
- 设置页提供打开备份目录。
- MVP 不提供备份列表或恢复页面。

## 15. 应用设置

```ts
type AppSettings = {
  ompExecutablePath?: string
  theme: "light" | "dark" | "system"
  selectedProviderId?: string
  selectedModelId?: string
  modelTestCostNoticeAccepted: boolean
}
```

不保存：

- Provider、模型或角色副本。
- API Key。
- 请求或响应正文。
- 任意 OMP 配置内容。

恢复默认设置只恢复应用设置，不删除 OMP 配置或备份。

## 16. 页面状态和交互

所有数据页面支持：

- Loading。
- Empty。
- Error。
- Normal。
- Read-only / Unsupported（适用时）。

表单：

- React Hook Form + Zod。
- Blur 后校验当前字段，提交时校验全部字段。
- 未修改或校验失败时禁用保存。
- 保存中禁用重复提交。
- 保存失败保留输入。
- 未保存离开时确认。

通知：

- Sonner 只显示摘要。
- 详细错误显示在页面、表单或弹窗。
- 错误说明发生了什么、可能原因和用户下一步。

动画：

- 仅轻量页面、Dialog、Sheet 和状态变化。
- 支持减少动画偏好。
- 视觉、布局与状态表现以 `designs/omp-switch.pen` 为强制实现契约；动效遵循 Flow 的轻量与减少动画规则。不得用组件库默认外观近似替代设计稿。

视觉实现规则：

- `00 Foundations` 定义颜色、字体、字号、字重、间距、圆角、边框和阴影 token。
- `01 Components` 定义按钮、输入框、选择器、状态、导航、卡片、页面标题、确认 Dialog 和表格行的共享外观。
- `02 Page / Setup Success`、`03 Page / Overview`、`04 Page / Providers List`、`05 Page / Provider Detail`、`06 Page / Roles Dirty`、`07 Page / Settings` 是对应页面的 1536×1024 视觉基准。
- `.pen` 中作为图片参考或嵌入状态呈现的 Provider 创建步骤、Model 创建 Sheet 也属于必须还原的视觉状态。
- 1:1 指几何结构与视觉 token 一致：布局、尺寸、对齐、间距、字体、字号、字重、颜色、圆角、边框、阴影、图标、文案、组件和状态层级不得自行改造。
- 响应式适配可以改变可用空间分配，但不得改变信息架构、视觉语言或组件比例关系；1536×1024 必须精确匹配设计基准。

Setup 页面按上述 issue #5 修订基准验收：1536×1024 使用无外层卡片布局，1100×720 最小窗口响应式收缩；该明确例外覆盖 `02 Page / Setup Success` 的最外层卡片几何和装饰，其余视觉规则不变。

## 17. 错误结构和日志

```ts
type AppError = {
  code: string
  message: string
  detail?: string
}
```

日志记录：

- 应用启动和退出。
- OMP 检测命令状态。
- 配置读取、验证、写入和冲突。
- 备份和事务恢复。
- 模型测试状态。
- 脱敏错误。

不得记录：

- API Key。
- 完整认证 Header。
- 完整请求和响应。
- OMP 配置正文。
- 无关本地文件内容。

## 18. Tauri 安全边界

前端不能：

- 执行任意 Shell 命令。
- 读取或写入任意路径。
- 获取 API Key。
- 指定任意配置目标。

Rust 只执行：

- 用户选择或 PATH 解析出的 OMP 可执行文件。
- 固定参数 `--version` 和 `config path`。
- 权威配置目录中的已确认文件操作。
- 固定协议的模型测试请求。
- 已确认应用数据备份目录操作。

## 19. 验收场景

### 19.1 首次使用

```text
选择或发现 OMP
→ 版本和配置路径成功
→ 创建缺失目录/文件（如需要）
→ 加载配置
→ 进入概览
```

### 19.2 创建 Provider 和模型

```text
打开“新增 Provider”
→ 填写 Provider 和首个模型
→ 选择 Provider 默认协议或模型协议
→ 一次保存完整 Provider 节点
→ 进入 Provider 详情
→ 手动测试
```

### 19.3 配置角色

```text
创建或选择角色
→ 选择 Provider/模型
→ 选择 Thinking Level
→ 保存 modelRoles 目标键
```

### 19.4 外部修改

```text
打开表单
→ 外部修改文件
→ 点击保存
→ Hash 冲突
→ 停止保存
→ 用户重新加载
```

### 19.5 跨文件事务恢复

```text
删除被角色引用的模型
→ 创建共享备份和事务清单
→ 第一文件替换后模拟崩溃
→ 应用重启
→ 保存现场副本
→ 恢复两个文件
→ 显示恢复结果
```

## 20. 发布前检查

发布阻断项：

- 三个平台矩阵全部通过。
- 四种协议的成功和错误路径通过。
- 所有 ID、协议、角色和 Token 规则通过。
- 高级和不支持配置只读分类通过。
- 当前 OMP 没有对应 bundled Provider 清单时，Provider/模型管理只读。
- 第一次模型测试显示费用说明。
- 内置角色不能改名或删除，自定义角色没有空角色状态。
- 未触及路径深度相等验证通过。
- 外部修改不会覆盖。
- 单文件失败不损坏原文件。
- 跨文件中断可确定恢复。
- 备份保留和隔离通过。
- API Key 不出现在 UI 返回、设置、IPC、日志或测试结果。
- `designs/omp-switch.pen` 中所有适用画板完成 1:1 还原，并有实现截图与 Pencil 导出截图的逐页对比证据。
- 不包含启动 OMP、终端、会话、工作目录或项目配置功能。
