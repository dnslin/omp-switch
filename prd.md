# OMP Switch 第一阶段 PRD

> 产品版本：v0.1  
> 文档状态：初稿  
> 产品形态：跨平台桌面应用  
> 第一阶段范围：Provider、模型、角色和 OMP 配置管理

---

## 1. 产品概述

### 1.1 产品背景

OMP 的 Provider、模型和角色配置主要通过 YAML 文件管理。

直接编辑 YAML 存在以下问题：

- 配置字段较多，容易写错。
- Provider、模型和角色之间存在引用关系。
- 修改 Provider ID 或模型 ID 后，需要同步更新角色。
- API Key 容易被误记录或暴露。
- 配置文件被外部修改时，容易出现覆盖。
- 配置写入失败时，可能损坏原文件。
- 测试模型连接需要手动构造请求。

OMP Switch 第一阶段提供桌面图形界面，帮助用户安全地管理这些配置。

---

### 1.2 产品定位

OMP Switch 是一个本地桌面配置工具，负责管理：

- OMP 环境状态。
- 自定义 Provider。
- Provider 下的模型。
- OMP 内置模型角色。
- 模型连接测试。
- OMP 配置文件。
- 配置备份。
- 应用基础设置。

OMP Switch 不替代 OMP，也不修改 OMP 的运行方式。

第一阶段不负责启动 OMP，不提供终端功能。

---

### 1.3 第一阶段目标

用户可以通过图形界面完成以下流程：

```text
检测 OMP
→ 读取已有配置
→ 创建自定义 Provider
→ 添加模型
→ 测试模型
→ 配置模型角色
→ 安全保存配置
```

第一阶段完成后，用户不需要直接编辑 `models.yml` 和 `config.yml`，即可完成日常 Provider、模型和角色配置。

---

## 2. 产品目标

### 2.1 核心目标

1. 降低 OMP 配置的使用门槛。
2. 减少 YAML 格式错误和字段填写错误。
3. 保证 Provider、模型和角色引用关系正确。
4. 提供模型连接测试，帮助用户快速定位配置问题。
5. 避免配置文件被错误覆盖或写坏。
6. 避免 API Key 出现在不必要的位置。
7. 兼容用户已有的 OMP 配置。

---

### 2.2 成功标准

第一阶段需要满足：

- 能够正确检测本机 OMP。
- 能够读取已有 OMP 配置。
- 能够新增、编辑、搜索和删除自定义 Provider。
- 能够新增、编辑、复制、搜索和删除模型。
- 能够测试三种 API 协议的模型连接。
- 能够配置全部十个内置角色。
- Provider 或模型改名后，角色引用能够同步更新。
- 保存失败时不会损坏原配置。
- 配置被其他程序修改后，不会被静默覆盖。
- 每次写入配置前都会创建备份。
- API Key 不会进入应用自己的配置和日志。
- 不包含启动 OMP 和终端相关能力。

---

## 3. 用户范围

### 3.1 目标用户

主要用户为：

- 使用 OMP 的开发者。
- 使用自定义 API 服务的用户。
- 使用 OpenAI Compatible、OpenAI Responses 或 Anthropic Compatible 服务的用户。
- 同时维护多个 Provider 和模型的用户。
- 不希望直接编辑 YAML 配置的用户。

---

### 3.2 典型使用场景

#### 场景一：第一次使用

用户启动 OMP Switch。

应用自动检测 OMP，读取 OMP 版本和配置目录。检测成功后，进入概览页。

如果没有配置文件，应用可以创建基础配置文件。

#### 场景二：添加自定义 Provider

用户填写 Provider ID、Base URL、协议和 API Key。

保存后，该 Provider 写入 OMP 配置。

#### 场景三：添加模型

用户进入 Provider 详情页，添加一个或多个模型，填写模型 ID、名称、上下文窗口和输出 Token 等信息。

#### 场景四：测试模型

用户选择一个模型执行连接测试。

应用返回：

- 是否成功。
- 请求耗时。
- HTTP 状态码。
- 失败原因。
- 简单的修复提示。

#### 场景五：配置角色

用户将某个模型分配给 `default`、`plan`、`vision` 等角色，并设置 Thinking Level。

#### 场景六：外部配置被修改

用户正在编辑 Provider 时，OMP 或其他编辑器修改了配置文件。

应用发现文件变化后，不直接覆盖，而是提示用户重新加载配置。

---

## 4. 第一阶段范围

### 4.1 包含内容

第一阶段包含：

- OMP 环境检测。
- 首次启动检查。
- 概览页面。
- 自定义 Provider 管理。
- 直接 API Key 管理。
- 三种 API 协议。
- 模型管理。
- 模型连接测试。
- 十个内置角色管理。
- YAML 配置读取和写入。
- 配置备份。
- 外部修改冲突检测。
- 应用主题和基础设置。
- 日志和错误处理。
- 基础自动化测试。

---

### 4.2 不包含内容

第一阶段不包含：

- 启动 OMP。
- 终端调用或终端管理。
- 工作目录选择。
- 会话管理。
- Provider 预设。
- 模型自动发现。
- OAuth 登录。
- 环境变量形式的 API Key。
- `!command` 形式的 API Key。
- Skills 管理。
- 项目级角色。
- 备份恢复页面。
- 在线模型市场。
- 自动更新。
- 用户登录和云同步。
- 数据库。

---

## 5. 产品结构

第一阶段包含以下页面：

```text
首次检查
概览
Providers
Provider 详情
角色
设置
```

侧边栏固定包含：

```text
概览
Providers
角色
设置
```

Provider 详情从 Providers 页面进入，不单独放在侧边栏中。

模型管理放在 Provider 详情内，不设置单独的顶层模型页面。

---

## 6. 核心用户流程

### 6.1 首次启动流程

```text
启动应用
→ 自动查找 OMP
→ 获取 OMP 版本
→ 获取配置目录
→ 检查配置文件
→ 检查目录读写权限
→ 加载配置
→ 进入概览页
```

检测失败时：

```text
显示失败原因
→ 用户手动选择 OMP
→ 重新检测
```

---

### 6.2 Provider 创建流程

```text
进入 Providers
→ 点击新增 Provider
→ 填写 Provider 信息
→ 表单校验
→ 保存
→ Rust 更新 models.yml
→ 创建备份
→ 返回 Provider 详情
```

---

### 6.3 模型创建流程

```text
进入 Provider 详情
→ 点击新增模型
→ 填写模型信息
→ 表单校验
→ 保存
→ 更新 models.yml
→ 显示在模型列表
```

---

### 6.4 模型测试流程

```text
选择模型
→ 点击测试
→ Rust 读取 Provider 和 API Key
→ 按协议构造最小请求
→ 发起请求
→ 返回测试结果
```

测试过程中用户可以取消请求。

---

### 6.5 角色配置流程

```text
进入角色页
→ 为角色选择 Provider 和模型
→ 设置 Thinking Level
→ 保存
→ 更新 config.yml
```

---

### 6.6 配置冲突流程

```text
用户打开编辑表单
→ 配置文件被外部修改
→ 用户点击保存
→ 应用发现文件 hash 已变化
→ 停止保存
→ 提示重新加载
```

第一阶段不处理复杂的自动合并。

---

## 7. 页面需求

## 7.1 首次检查页面

### 页面目标

帮助用户完成 OMP 环境确认。

### 页面内容

- OMP 检测状态。
- OMP 可执行文件路径。
- OMP 版本。
- OMP 配置目录。
- 配置目录读写状态。
- `models.yml` 状态。
- `config.yml` 状态。
- 自动检测按钮。
- 手动选择 OMP 按钮。
- 重新检测按钮。
- 打开配置目录按钮。

### 页面状态

#### 检测中

显示加载状态，并禁用重复检测。

#### 检测成功

显示：

- OMP 版本。
- OMP 路径。
- 配置目录。
- 配置文件状态。

用户可以进入应用。

#### 未找到 OMP

显示：

- 未找到 OMP 的说明。
- 手动选择 OMP 按钮。
- 重新检测按钮。

#### 配置异常

显示具体问题：

- 配置目录不存在。
- 配置目录不可读。
- 配置目录不可写。
- 配置文件格式错误。
- OMP 版本过低。

---

## 7.2 概览页面

### 页面目标

让用户快速查看当前 OMP 配置状态，并测试常用模型。

### 页面内容

#### OMP 状态

- 是否已检测。
- OMP 版本。
- OMP 路径。
- 配置目录状态。

#### 配置统计

- Provider 数量。
- 模型数量。
- 已配置角色数量。
- 无效角色数量。

#### 当前模型

- Provider。
- 模型。
- API 协议。
- Context Window。
- Max Tokens。
- 是否支持 Reasoning。
- 支持的输入类型。

#### 快速操作

- 选择 Provider。
- 选择模型。
- 测试模型。
- 前往 Provider 详情。
- 前往角色配置。

#### 最近一次测试

- 测试时间。
- 测试状态。
- 请求耗时。
- HTTP 状态码。
- 错误摘要。

概览页不提供启动 OMP 按钮。

---

## 7.3 Providers 页面

### 页面目标

查看和管理全部自定义 Provider。

### 页面内容

- 页面标题。
- Provider 搜索框。
- 新增 Provider 按钮。
- Provider 列表。
- 空状态。
- 加载状态。
- 错误状态。

### Provider 列表项

每个 Provider 显示：

- Provider ID。
- Base URL。
- API 协议。
- 是否已配置 API Key。
- 模型数量。
- 配置状态。
- 编辑入口。
- 删除入口。

### Provider 状态

可以显示以下状态：

- 正常。
- 缺少 API Key。
- 没有模型。
- 配置无效。
- 包含应用未识别的字段。

这些状态只用于提示，不自动修改用户配置。

---

## 7.4 Provider 新增和编辑

### 页面形式

Provider 可以使用独立页面或较大的对话框。

考虑到 Provider 详情还包含模型管理，建议使用独立详情页面。

### Provider 字段

```ts
type ProviderForm = {
  id: string
  baseUrl: string
  api:
    | "openai-completions"
    | "openai-responses"
    | "anthropic-messages"
  authMode: "api-key" | "none"
  apiKey?: string
}
```

### 字段说明

#### Provider ID

- 必填。
- 同一个配置中不能重复。
- 用于组成完整模型 ID。
- 修改后需要同步更新角色引用。
- 不能只包含空格。
- 不允许包含明显会破坏配置结构的字符。

#### Base URL

- 必填。
- 必须是有效 URL。
- 支持 `http` 和 `https`。
- 本地服务可以使用 `http`。
- 保存时可以去除末尾多余空格。
- 不应擅自修改用户填写的路径部分。

#### API 协议

支持：

- `openai-completions`
- `openai-responses`
- `anthropic-messages`

#### 认证方式

支持：

- API Key。
- 不需要认证。

#### API Key

- 只支持直接填写密钥。
- 新建 Provider 时可以填写。
- 编辑 Provider 时不显示原密钥。
- 留空表示保留原密钥。
- 可以选择替换密钥。
- 可以单独删除密钥。

### 页面操作

- 保存。
- 取消。
- 删除 Provider。
- 删除 API Key。
- 查看模型列表。
- 新增模型。

---

## 7.5 Provider 删除

删除 Provider 前必须显示确认对话框。

确认内容包含：

- Provider ID。
- Provider 下的模型数量。
- 受影响的角色。
- 删除后不可直接撤销的说明。

删除 Provider 后：

- 删除 Provider 及其模型配置。
- 清理或标记受影响角色。
- 第一阶段建议清除受影响的内置角色映射。
- 保留其他应用不认识的配置字段。
- 保存前创建备份。

用户取消时不进行任何修改。

---

## 7.6 模型列表

### 页面位置

模型列表显示在 Provider 详情页。

### 页面内容

- 模型搜索框。
- 新增模型按钮。
- 模型列表。
- 模型数量。
- 空状态。
- 排序功能。

### 模型列表项

每个模型显示：

- 模型名称。
- 模型 ID。
- 完整标识 `provider/model`。
- 是否支持 Reasoning。
- 输入类型。
- Context Window。
- Max Tokens。
- 被引用的角色数量。
- 测试按钮。
- 编辑按钮。
- 复制按钮。
- 删除按钮。

---

## 7.7 模型新增和编辑

### 模型字段

```ts
type ModelForm = {
  id: string
  name: string
  reasoning: boolean
  input: Array<"text" | "image">
  contextWindow: number
  maxTokens: number
}
```

### 字段规则

#### 模型 ID

- 必填。
- 同一个 Provider 下不能重复。
- 修改后需要同步更新角色引用。

#### 模型名称

- 必填。
- 用于界面显示。
- 可以与模型 ID 相同。

#### Reasoning

表示模型是否支持推理能力。

#### 输入类型

至少选择一种：

- Text。
- Image。

一般模型默认选择 Text。

#### Context Window

- 必填。
- 必须是正整数。
- 不允许为零或负数。

#### Max Tokens

- 必填。
- 必须是正整数。
- 不应明显超过 Context Window。
- 超过时需要阻止保存或显示明确警告。

### 模型默认协议

第一阶段模型继承 Provider 的 API 协议。

模型不能单独覆盖协议。

---

## 7.8 模型复制

用户点击复制模型后：

- 复制原模型全部普通字段。
- 不复制任何测试结果。
- 自动生成一个不冲突的临时模型 ID。
- 打开模型编辑表单。
- 用户确认保存后才写入配置。

复制过程不立即修改配置文件。

---

## 7.9 模型删除

删除模型前显示：

- 模型名称。
- 模型 ID。
- 所属 Provider。
- 受影响的角色。

删除后：

- 从 Provider 中删除该模型。
- 清除或标记相关角色映射。
- 保存前创建备份。

---

## 7.10 角色页面

### 页面目标

管理 OMP 内置模型角色。

### 支持角色

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

### 每个角色显示

- 角色名称。
- 角色说明。
- 当前 Provider。
- 当前模型。
- 完整模型 ID。
- Thinking Level。
- 当前状态。
- 清除按钮。

### 页面操作

- 保存全部角色。
- 清除单个角色。
- 清除全部内置角色。
- 将当前模型设为 `default`。
- 放弃未保存修改。

### 角色状态

- 正常。
- 未配置。
- Provider 不存在。
- 模型不存在。
- 配置格式无法识别。

### 未识别角色

如果 `config.yml` 中存在应用不认识的自定义角色：

- 不删除。
- 不覆盖。
- 第一阶段可以只读显示。
- 保存内置角色时保留这些配置。

---

## 7.11 Thinking Level

Thinking Level 的可选值应根据 OMP 当前支持的配置确定。

前端可以提供默认选项，但数据结构不能限制未来增加新值。

如果读取到未知 Thinking Level：

- 保留原值。
- 显示为自定义值或未知值。
- 不在用户没有修改该角色时覆盖。

---

## 7.12 设置页面

### OMP 设置

- 自定义 OMP 可执行文件路径。
- 恢复使用系统 `PATH`。
- 重新检测 OMP。
- 显示 OMP 版本。
- 显示 OMP 配置目录。
- 打开 OMP 配置目录。

### 主题设置

- 浅色。
- 深色。
- 跟随系统。

### 应用信息

- 应用版本。
- 打开应用配置目录。
- 打开应用日志目录。
- 恢复默认设置。

### 恢复默认设置

恢复内容包括：

- 主题。
- 自定义 OMP 路径。
- 当前选择的 Provider。
- 当前选择的模型。
- 页面相关的本地界面设置。

恢复默认设置不会删除：

- OMP Provider。
- OMP 模型。
- OMP 角色。
- OMP API Key。
- OMP 配置文件。
- 自动备份文件。

---

## 8. OMP 环境检测

### 8.1 自动检测

应用启动时优先使用：

1. 用户已经设置的 OMP 可执行文件路径。
2. 系统 `PATH` 中的 `omp`。

检测时执行：

```text
omp --version
omp config path
```

### 8.2 检测结果

```ts
type OmpEnvironmentStatus = {
  installed: boolean
  executablePath?: string
  version?: string
  configDir?: string
  modelsFileExists: boolean
  configFileExists: boolean
  readable: boolean
  writable: boolean
  supported: boolean
  error?: AppError
}
```

### 8.3 配置文件不存在

当 `models.yml` 或 `config.yml` 不存在时：

- 提示用户文件不存在。
- 提供创建基础配置文件的操作。
- 创建前进行确认。
- 只创建最小有效结构。
- 不覆盖已经存在的文件。

### 8.4 配置格式错误

格式错误时显示：

- 文件名称。
- 大致行号和列号。
- 简单错误说明。
- 打开配置目录按钮。
- 重新读取按钮。

存在格式错误时，不允许继续写入对应文件。

---

## 9. API Key 管理规则

### 9.1 保存方式

API Key 按照 OMP 的配置格式保存在 OMP 配置文件中。

OMP Switch 不在应用自身配置中保存 API Key 副本。

### 9.2 前端行为

Rust 返回 Provider 信息时：

```ts
type ProviderView = {
  id: string
  baseUrl: string
  api: string
  authMode: "api-key" | "none"
  hasApiKey: boolean
  modelCount: number
}
```

不返回实际 API Key。

### 9.3 编辑规则

- API Key 输入框默认为空。
- 输入为空表示保留原 API Key。
- 输入新值表示替换 API Key。
- 删除 API Key 使用单独操作。
- 切换认证方式为 `none` 时，需要询问是否删除已有 API Key。
- 前端提交完成后清空表单中的 API Key。

### 9.4 禁止行为

API Key 不允许进入：

- Zustand 持久化状态。
- localStorage。
- sessionStorage。
- 应用自身设置文件。
- 前端日志。
- Rust 日志。
- 错误详情。
- Sonner 提示。
- Tauri 事件广播。
- 模型测试历史。

---

## 10. API 协议

第一阶段支持三种协议。

### 10.1 OpenAI Completions

协议值：

```text
openai-completions
```

测试时需要处理：

- 请求地址。
- Bearer Token。
- 最小消息体。
- 模型 ID。
- 返回文本。
- 标准 HTTP 错误。

---

### 10.2 OpenAI Responses

协议值：

```text
openai-responses
```

测试时使用 Responses API 对应的最小请求格式。

不能直接复用 Completions 的请求体。

---

### 10.3 Anthropic Messages

协议值：

```text
anthropic-messages
```

测试时使用 Anthropic Messages 对应的：

- API Key Header。
- 版本 Header。
- 请求体。
- 响应解析。

---

### 10.4 协议实现要求

每种协议需要有独立的：

- 请求地址生成逻辑。
- 请求头生成逻辑。
- 请求体生成逻辑。
- 响应解析逻辑。
- 错误处理逻辑。

前端不直接拼接协议请求。

---

## 11. 模型连接测试

### 11.1 测试入口

模型测试可以从以下位置发起：

- Provider 详情页模型列表。
- 模型编辑页。
- 概览页。

### 11.2 测试参数

前端只提交：

```ts
type ModelTestInput = {
  providerId: string
  modelId: string
  timeoutSeconds?: number
}
```

Rust 根据 Provider 和模型 ID 读取实际配置。

前端不提交已经保存的 API Key。

### 11.3 测试结果

```ts
type ModelTestResult = {
  success: boolean
  providerId: string
  modelId: string
  protocol: string
  latencyMs: number
  status?: number
  message: string
  errorCode?: string
}
```

### 11.4 测试行为

- 显示测试中状态。
- 防止同一个模型重复发起测试。
- 支持请求超时。
- 支持主动取消。
- 测试完成后显示页面结果。
- 同时使用 Sonner 提示成功或失败。
- 不保存完整响应正文。
- 不把 API Key 返回前端。

### 11.5 错误类型

需要区分：

| 错误 | 用户提示方向 |
|---|---|
| Base URL 无效 | 检查 Provider 地址 |
| DNS 失败 | 检查域名或网络 |
| 连接失败 | 检查服务是否启动 |
| TLS 错误 | 检查证书和 HTTPS 配置 |
| 请求超时 | 检查网络或增加超时时间 |
| 401 | 检查 API Key |
| 403 | 检查密钥权限 |
| 404 | 检查接口路径或模型 ID |
| 429 | 请求过于频繁或额度不足 |
| 5xx | Provider 服务异常 |
| 响应格式错误 | 当前接口可能不兼容所选协议 |
| 用户取消 | 测试已取消 |

---

## 12. 配置文件管理

### 12.1 数据来源

第一阶段不使用数据库。

业务数据来源：

| 数据 | 来源 |
|---|---|
| Provider | `models.yml` |
| 模型 | `models.yml` |
| 角色 | `config.yml` |
| API Key | OMP 配置 |
| 应用主题 | 应用设置 |
| 自定义 OMP 路径 | 应用设置 |
| 当前选择项 | 应用设置 |

OMP 配置文件是 Provider、模型和角色数据的唯一来源。

---

### 12.2 配置读取

Rust 负责：

- 获取实际配置路径。
- 读取 YAML。
- 解析 Provider。
- 解析模型。
- 解析角色。
- 保存读取时的文件 hash。
- 保留未知字段。
- 返回适合前端展示的数据。

前端不能读取任意配置文件路径。

---

### 12.3 配置写入

写入流程：

```text
重新读取当前文件
→ 比较文件 hash
→ 创建备份
→ 修改目标内容
→ 验证完整配置
→ 写入临时文件
→ 重新读取临时文件
→ 原子替换原文件
```

### 12.4 保存要求

- 只修改用户实际操作的内容。
- 不删除应用不认识的字段。
- 写入失败时保留原文件。
- 备份失败时不继续写入。
- 临时文件验证失败时不替换原文件。
- 外部修改后不直接覆盖。
- 保存成功后重新读取配置。
- 前端展示的数据以重新读取结果为准。

### 12.5 YAML 格式说明

第一阶段保证：

- 配置含义不丢失。
- 未知字段尽量保留。
- 配置可以被 OMP 正常读取。

第一阶段不保证：

- 原有注释完全保留。
- 空行完全保留。
- 字段顺序完全不变。
- 缩进格式完全不变。

---

## 13. Provider 和模型引用规则

### 13.1 Provider ID 修改

修改 Provider ID 时：

- 检查新 ID 是否冲突。
- 更新 Provider ID。
- 更新完整模型引用。
- 更新内置角色引用。
- 显示受影响的角色数量。
- 操作失败时不保存任何部分。

### 13.2 模型 ID 修改

修改模型 ID 时：

- 检查同一个 Provider 下是否冲突。
- 更新模型 ID。
- 更新内置角色引用。
- 显示受影响的角色。
- 操作失败时不保存任何部分。

### 13.3 删除 Provider

删除 Provider 时：

- 删除 Provider。
- 删除 Provider 下的模型。
- 清除受影响的内置角色。
- 保留与该 Provider 无关的未知配置。
- 写入前创建备份。

### 13.4 删除模型

删除模型时：

- 删除该模型。
- 清除受影响的内置角色。
- 保留其他模型和未知字段。

---

## 14. 配置备份

### 14.1 备份时机

以下操作写入前创建备份：

- 新增 Provider。
- 编辑 Provider。
- 删除 Provider。
- 修改 API Key。
- 新增模型。
- 编辑模型。
- 复制模型。
- 删除模型。
- 保存角色。
- 清除角色。

### 14.2 备份规则

- `models.yml` 和 `config.yml` 分别备份。
- 文件名包含日期和时间。
- 自动保留最近 10 份。
- 超出后删除最旧备份。
- 备份保存到应用管理的备份目录。
- 备份失败时中止保存。

第一阶段不提供备份查看和恢复页面。

用户可以通过应用配置目录找到备份文件。

---

## 15. 外部修改和冲突处理

### 15.1 冲突判断

应用读取配置时记录文件 hash。

保存前重新读取文件，并比较 hash。

如果 hash 不一致，说明文件已被其他程序修改。

### 15.2 冲突提示

提示内容包括：

- 哪个文件发生了变化。
- 当前修改不能直接保存。
- 重新加载会丢失当前未保存内容。

提供操作：

- 取消。
- 重新加载配置。

第一阶段不提供自动合并。

---

## 16. 应用设置

应用设置结构：

```ts
type AppSettings = {
  ompExecutablePath?: string
  theme: "light" | "dark" | "system"
  selectedProviderId?: string
  selectedModelId?: string
}
```

### 保存内容

应用可以保存：

- OMP 可执行文件路径。
- 主题。
- 当前选中的 Provider。
- 当前选中的模型。
- 必要的界面状态。

### 不保存内容

应用不保存：

- 完整 Provider 数据。
- 完整模型数据。
- 角色配置副本。
- API Key。
- 模型测试响应。
- 用户项目内容。

---

## 17. 页面交互要求

### 17.1 通用状态

所有数据页面需要支持：

- Loading。
- Empty。
- Error。
- Normal。

### 17.2 表单

- 使用 React Hook Form 管理表单。
- 使用 Zod 校验。
- 错误显示在对应字段附近。
- 内容没有变化时禁用保存。
- 保存过程中禁用重复提交。
- 保存成功后显示 Sonner。
- 保存失败后显示可读错误。

### 17.3 未保存修改

Provider、模型和角色表单存在未保存修改时：

- 页面显示未保存状态。
- 离开页面前弹出确认。
- 关闭编辑对话框前弹出确认。
- API Key 不参与草稿恢复。

### 17.4 删除操作

所有删除操作使用统一确认对话框。

危险按钮需要与普通操作有明显区别。

### 17.5 动画

Motion 只用于：

- 页面切换。
- 对话框。
- 展开和收起。
- 状态变化。

不使用影响操作效率的大面积动画。

---

## 18. UI 设计原则

整体界面以桌面软件为主，不使用过强的网页风格。

设计要求：

- 界面接近 macOS 桌面软件的简洁风格。
- 信息密度适中。
- 操作入口清楚。
- 不使用过多卡片嵌套。
- 不使用大面积渐变和装饰。
- 表单宽度合理，不铺满整个窗口。
- Provider 和模型列表适合快速浏览。
- 危险操作保持克制但明确。
- 深色和浅色主题都需要完整适配。
- 窗口缩小时保持基本可用。

---

## 19. 错误处理

统一错误结构：

```ts
type AppError = {
  code: string
  message: string
  detail?: string
}
```

### 错误展示原则

用户看到的错误信息需要说明：

1. 发生了什么。
2. 可能的原因。
3. 用户可以怎么处理。

例如：

```text
无法连接 Provider

没有连接到 https://api.example.com。
请检查 Base URL、网络连接或服务是否已经启动。
```

不应只显示：

```text
Request failed
```

---

## 20. 日志

### 20.1 日志内容

第一阶段记录：

- 应用启动。
- OMP 检测。
- 配置读取。
- 配置写入。
- 配置备份。
- 配置冲突。
- 模型测试状态。
- 应用错误。

### 20.2 日志限制

不得记录：

- API Key。
- 完整认证 Header。
- 完整请求体。
- 完整模型响应。
- 用户项目文件。
- 与当前功能无关的本地文件内容。

日志中的敏感字段必须脱敏。

---

## 21. 安全要求

### 21.1 Tauri 权限

前端只获得当前功能需要的权限。

不向前端开放：

- 任意 Shell 命令执行。
- 任意文件读取。
- 任意文件写入。
- 任意目录打开。
- 任意网络密钥读取。

### 21.2 Rust 职责

以下能力只能在 Rust 中实现：

- OMP 路径检测。
- 执行固定的 OMP 状态命令。
- 读取 OMP 配置。
- 写入 OMP 配置。
- 配置备份。
- 模型连接测试。
- API Key 读取。
- 打开已确认的配置目录。

### 21.3 前端职责

前端只负责：

- 界面展示。
- 表单输入。
- 表单基础校验。
- 调用固定的 Tauri Command。
- 展示 Rust 返回的数据和错误。

---

## 22. 技术约束

| 层级 | 技术 |
|---|---|
| 桌面框架 | Tauri 2 |
| 后端 | Rust |
| 前端 | React |
| 语言 | TypeScript Strict Mode |
| 构建 | Vite |
| UI | shadcn/ui |
| 样式 | Tailwind CSS |
| 状态 | Zustand |
| 表单 | React Hook Form |
| 校验 | Zod |
| 表单适配 | `@hookform/resolvers` |
| 图标 | lucide-react |
| 动画 | Motion |
| 提示 | Sonner |
| 前端测试 | Vitest + Testing Library |
| Rust 测试 | Cargo test |
| 包管理 | pnpm |

第一阶段不引入：

- 数据库。
- ORM。
- 完整状态同步框架。
- 嵌入式终端。
- 多窗口架构。
- OMP Sidecar。

---

## 23. 建议的 Rust 模块

```text
environment
config
provider
model
role
model_test
backup
settings
logging
error
```

### 模块职责

#### environment

- 检测 OMP。
- 获取版本。
- 获取配置路径。
- 检查文件权限。

#### config

- 读取 YAML。
- 保留未知字段。
- 校验配置。
- 安全写入。
- 检测冲突。

#### provider

- Provider 新增、编辑和删除。
- Provider ID 修改。
- API Key 更新和删除。

#### model

- 模型新增、编辑、复制和删除。
- 模型 ID 修改。
- 引用检查。

#### role

- 读取和保存角色。
- 清除角色。
- 检查无效引用。

#### model_test

- 构建三种协议请求。
- 执行请求。
- 取消请求。
- 处理错误。

#### backup

- 创建备份。
- 清理旧备份。

#### settings

- 保存应用设置。
- 恢复默认设置。

---

## 24. 建议的前端模块

```text
pages/
  overview
  providers
  provider-detail
  roles
  settings
  setup

features/
  environment
  providers
  models
  roles
  model-test
  settings

components/
  layout
  forms
  states
  dialogs
```

Zustand 只保存轻量界面状态。

Provider、模型和角色数据每次从 Rust 获取，不作为长期前端数据源。

---

## 25. 测试要求

### 25.1 Rust 测试

需要覆盖：

- OMP 检测。
- Provider YAML 解析。
- 模型 YAML 解析。
- 角色 YAML 解析。
- 未知字段保留。
- Provider 新增、编辑和删除。
- 模型新增、编辑、复制和删除。
- Provider ID 修改后的引用更新。
- 模型 ID 修改后的引用更新。
- 配置 hash 冲突检测。
- 配置备份。
- 临时文件写入。
- 原子替换。
- API Key 脱敏。
- 三种协议请求构造。
- 三种协议响应解析。
- 网络错误分类。
- 请求超时。
- 请求取消。

### 25.2 前端测试

需要覆盖：

- Provider 表单校验。
- 模型表单校验。
- API Key 编辑规则。
- Provider 增删改。
- 模型增删改和复制。
- 删除确认对话框。
- 角色选择和清除。
- 未保存修改提示。
- 模型测试加载状态。
- 模型测试取消。
- 空状态。
- 错误状态。
- Sonner 提示。

---

## 26. 第一阶段验收清单

### OMP 环境

- [ ] 可以自动检测 OMP。
- [ ] 可以手动选择 OMP。
- [ ] 可以显示 OMP 版本。
- [ ] 可以获取实际配置目录。
- [ ] 可以检查配置目录读写权限。
- [ ] 配置文件错误时可以显示位置。

### Provider

- [ ] 可以新增自定义 Provider。
- [ ] 可以编辑 Provider。
- [ ] 可以搜索 Provider。
- [ ] 可以删除 Provider。
- [ ] 可以选择三种支持协议。
- [ ] 可以使用直接 API Key。
- [ ] 可以使用无认证 Provider。
- [ ] Provider ID 冲突时不能保存。

### API Key

- [ ] 编辑时不返回原 API Key。
- [ ] 留空可以保留旧 API Key。
- [ ] 可以替换 API Key。
- [ ] 可以删除 API Key。
- [ ] API Key 不进入应用设置和日志。

### 模型

- [ ] 可以新增模型。
- [ ] 可以编辑模型。
- [ ] 可以复制模型。
- [ ] 可以搜索模型。
- [ ] 可以删除模型。
- [ ] 可以配置模型能力。
- [ ] 模型 ID 冲突时不能保存。

### 模型测试

- [ ] 可以测试 OpenAI Completions。
- [ ] 可以测试 OpenAI Responses。
- [ ] 可以测试 Anthropic Messages。
- [ ] 可以显示耗时和状态码。
- [ ] 可以取消测试。
- [ ] 可以区分常见网络和认证错误。

### 角色

- [ ] 可以配置全部十个角色。
- [ ] 可以设置 Thinking Level。
- [ ] 可以清除单个角色。
- [ ] 可以清除全部角色。
- [ ] 可以识别失效模型。
- [ ] Provider 或模型改名后引用可以更新。

### 配置安全

- [ ] 写入前会创建备份。
- [ ] 备份失败时不会继续写入。
- [ ] 写入失败时不会损坏原文件。
- [ ] 外部修改配置后不会静默覆盖。
- [ ] 应用不认识的字段不会被直接删除。

### 范围确认

- [ ] 不包含启动 OMP。
- [ ] 不包含终端功能。
- [ ] 不包含工作目录。
- [ ] 不包含会话管理。
- [ ] 不包含模型自动发现。
- [ ] 不包含 Skills。
- [ ] 不包含 OAuth。
- [ ] 不包含环境变量 API Key。

---

## 27. 开发顺序建议

### 阶段一：应用基础

- 创建 Tauri 和 React 工程。
- 集成 UI、表单、状态和提示组件。
- 完成主布局和页面路由。
- 完成统一错误结构。

### 阶段二：OMP 环境

- 检测 OMP。
- 获取版本和配置目录。
- 检查配置文件。
- 完成首次检查页面。

### 阶段三：配置读取

- 读取 `models.yml`。
- 读取 `config.yml`。
- 转换前端 DTO。
- 保留未知字段。

### 阶段四：Provider

- Provider 列表。
- Provider 新增和编辑。
- API Key 管理。
- Provider 删除。

### 阶段五：模型

- 模型列表。
- 新增和编辑。
- 搜索和排序。
- 复制和删除。

### 阶段六：模型测试

- 三种协议适配。
- 超时和取消。
- 错误分类。
- 测试结果展示。

### 阶段七：角色

- 十个内置角色。
- Thinking Level。
- 无效引用检查。
- 引用迁移。

### 阶段八：安全写入

- 文件 hash。
- 自动备份。
- 临时文件写入。
- 原子替换。
- 外部修改冲突。

### 阶段九：设置和体验

- 主题。
- OMP 路径。
- 目录入口。
- 未保存提示。
- 页面状态和错误提示。

### 阶段十：测试和验收

- Rust 单元测试。
- 前端组件测试。
- 真实 OMP 配置验收。
- API Key 和日志安全检查。

---

## 28. 上线前需要确认的内容

以下内容需要在进入开发前，根据实际 OMP 配置再确认：

1. 最低支持的 OMP 版本。
2. 当前 OMP 的完整 Thinking Level 可选值。
3. `models.yml` 中 Provider 和模型的准确字段结构。
4. `config.yml` 中角色配置的准确字段位置。
5. 三种协议的 Base URL 拼接规则。
6. Anthropic 协议使用的版本 Header。
7. Provider 删除时，受影响角色是自动清除还是阻止删除。
8. YAML 库对未知字段、字段顺序和注释的保留能力。

其中第 7 项建议采用：

> 删除前显示受影响角色，用户确认后删除 Provider，并自动清除对应的内置角色配置。

这样流程更直接，也避免留下失效引用。