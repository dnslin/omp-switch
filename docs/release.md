# OMP Switch 发布验收

本文记录 issue #16 的可复核发布入口、支持矩阵、验证命令和证据位置。产品范围仍由 `docs/mvp.md`、`docs/prd.md`、`docs/flow.md` 及 `designs/omp-switch.pen` 约束；本文不增加产品功能。

## 前置条件

issue #16 的全部 GitHub blockers 已关闭：#7、#8、#9、#10、#11、#12、#13、#14、#15。GitHub 原生依赖摘要显示 #16 的 `blocked_by` 为 0。

发布分支必须使用原生构建环境。禁止用 `--target` 在一个平台上交叉编译另一个平台；每个 runner 只构建自己的 Rust host target。

## 原生构建矩阵

| 产物 | GitHub Actions runner | 架构 | 主要 bundle |
| --- | --- | --- | --- |
| macOS Intel | `macos-15-intel` | x86_64 | `.app`、`.dmg` |
| macOS Apple Silicon | `macos-14` | arm64 | `.app`、`.dmg` |
| Windows | `windows-2022` | x64 | `.msi`、`.exe` |
| Ubuntu 22.04 | `ubuntu-22.04` | x64 | `.deb`、`.AppImage`（唯一 Linux 构建） |
| Ubuntu 24.04 runtime | `ubuntu-24.04` | x64 | 启动同一份 Ubuntu 22.04 `.AppImage` |

Linux 只在 Ubuntu 22.04 构建一次，以最老支持系统控制 glibc；Ubuntu 24.04 job 只下载并启动同一份收集后的 `.AppImage`，不重新构建或发布第二套 amd64 bundle。

macOS 构建显式设置 `MACOSX_DEPLOYMENT_TARGET=13.0`，并在 `tauri.conf.json` 写入 `bundle.macOS.minimumSystemVersion=13.0`。runner 只构建自己的 Rust host target，不使用交叉编译参数。
Windows 安装器使用 Tauri `webviewInstallMode.type=skip`；正式支持的 Windows 10 22H2/11 已随系统提供 WebView2 Runtime。`windows-2022` 是 Server runner，workflow 在安装 NSIS 前显式安装 Evergreen Runtime，不能把 Server runner 当作 Windows 客户端版本声明。

## 构建前校验

```bash
pnpm install --frozen-lockfile
pnpm test:manifest
pnpm typecheck
pnpm exec vitest run
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build -- --verbose
```

`pnpm test:manifest` 验证官方 `@oh-my-pi/pi-catalog@17.2.15` 的精确版本和规范源文件；Rust `build.rs` 再验证每个资源文件名、声明版本和 Provider/model 结构。清单缺失或不一致时构建失败。

## 高层 seam 验收

Rust application-service seam 使用临时 Target configuration、受控 OMP executable、临时 HTTP server 和故障注入，覆盖：

- OMP PATH/手动检测、`omp --version`、`omp config path` 及失败不猜路径。
- `.yml`、`.yaml`、旧 JSON、缺失目录/文件、权限、符号链接和真实目标。
- 脱敏读取、Provider/Model/role CRUD、Stable ID、只读分类和四协议请求。
- Hash 冲突、单文件失败、备份、事务中断/恢复、备份隔离/保留和清理告警。
- Direct API Key 不进入 DTO、设置、IPC、日志、通知或测试结果。

React page seam 使用真实 routed pages、可访问控件和 typed Tauri client substitute，覆盖页面状态、表单校验、未保存确认、冲突恢复、事务恢复、单并发模型测试、取消、超时和敏感值不显示。

## 平台真实验收

- macOS 安装包：在当前 Apple Silicon 主机运行 `MACOSX_DEPLOYMENT_TARGET=13.0 pnpm tauri build`，并在 `macos-14` workflow 复验；Intel 由 `macos-15-intel` 原生 runner 构建和启动。最低系统版本由 bundle metadata 固定为 macOS 13.0。
- Windows x64：在 `windows-2022` 原生 runner 构建、安装、启动并执行 Rust/React seam。
- Ubuntu 22.04/24.04 x64：Ubuntu 22.04 构建并执行 Rust/React seam；Ubuntu 24.04 下载并启动同一份 Ubuntu 22.04 `.AppImage`，smoke JSON 记录正式资产名。
- 每个平台的 runner 日志、bundle、smoke JSON 和两组九页真实 Tauri UI 证据作为 workflow artifacts 保存 14 天。`viewport/` 由该平台刚构建、使用正式 `src-tauri/tauri.conf.json` 原生装饰配置的 Tauri binary 生成，固定 `window.innerWidth === 1536 && window.innerHeight === 1024`，满足 issue #16 的 1536×1024 视口验收；`content/` 固定 1536×960，用于去掉 Pencil 64px 伪标题栏后的逐页比较。两组都不是浏览器或静态页面截图；`tauri-plugin-wdio-webdriver` 的 macOS endpoint 使用 `WKWebView.takeSnapshot`，PNG 不包含 OS 原生标题栏/交通灯。每次运行保留 `raw-snapshot.png` 与 `window-geometry.json`，后者记录请求的原生窗口基线、实际外窗物理尺寸、DPR、DOM 内容视口和 raw snapshot 尺寸。

原生应用窗口标题由 `src-tauri/tauri.conf.json` 的 Tauri window `title` 提供，正式装饰和标题由 packaged-app smoke/启动配置验证；页面不绘制第二套标题栏。视觉比较使用 `content/` 的 1536×960 PNG 与 Pencil 含背景的内容基准，不把 OS 原生控件或 Pencil `Window Title Bar` 节点计入比较。若 raw snapshot 比目标内容多出底部像素，归一化只在运行时证明整带 RGBA 同色背景后排除；非均匀额外像素直接失败。OS chrome 高度跨平台不同，不宣称统一的实际外窗高度；Setup 的内容画板使用 issue #5 已批准的无外层卡片基准（`.artifacts/issue-5/implementation-1536x1024.png`），仅保留检测表格、状态行、间距和操作区。


## 视觉证据

视觉权威是 Pencil 文件的以下节点：

- `R6EUPs`：Setup Success
- `chFfk`：Overview
- `H6IsaW`：Providers List
- `vz6sg`：Provider Detail
- `i5xFP`：Roles Dirty
- `W7copJ`：Settings
- `N3OTR` / `r67daM`：Foundations / Components

Provider 创建两步和 Model Create Sheet 必须使用对应 `.pen` 的已填写可提交状态：Step 1 的 Provider ID、Base URL、API Key 已填且“下一步”可用；Step 2 的 Model ID、名称已填、最终地址已解析且“创建 Provider”可用；Model Sheet 的 Model ID、名称、显式协议已填且“保存模型”可用。每个 matrix 平台必须在 `viewport/` 产生以下九个 1536×1024 PNG：`setup-success.png`、`overview.png`、`providers-list.png`、`provider-detail.png`、`roles-dirty.png`、`settings.png`、`provider-create-step1.png`、`provider-create-step2.png`、`model-create-sheet.png`；并在 `content/` 产生同名九个 1536×960 Pencil 内容比较 PNG。两组各自保留 `raw-snapshot.png`、`window-geometry.json` 和每页 `.normalization.json`，artifact 名称为 `omp-switch-<matrix-id>-ui`。`.normalization.json` 必须证明任何被排除的底部 snapshot 带是整带同色背景；非均匀额外像素直接失败。逐页比较记录必须随发布证据保存；未批准的肉眼可见差异不能通过文档豁免。

## 发布命令

```bash
gh workflow run platform-release.yml --ref <release-branch>
gh run list --workflow platform-release.yml
gh run watch <run-id>
```

验证所有 matrix job、bundle artifact、smoke JSON、Rust/React 测试和 manifest 校验均成功后，才可将 draft release 提交人工发布。

## 已完成发布证据

- blocker gate：`gh api repos/dnslin/omp-switch/issues/16 --jq '{state,blocked_by:(.dependencies_summary.blocked_by // .issue_dependencies_summary.blocked_by // null)}'` 返回 `{"state":"open","blocked_by":0}`；#7–#15 均已关闭。
- 修正版原生矩阵：workflow run `32370762456`，commit `13a90e44c42fef1495bef2e7185ee7b949dd6657`，结论 `success`。macOS arm64、macOS Intel、Windows x64、Ubuntu 22.04 x64 和 Ubuntu 24.04 复用 Ubuntu 22.04 AppImage 的 job 全部成功；证据已下载到 `.artifacts/issue-16/ci-32370762456/`。
- bundle 清单：macOS arm64 `.dmg`、macOS Intel `.dmg`、Windows `.msi`/NSIS `.exe`、Ubuntu 22.04 `.deb`/`.AppImage`/`.rpm` 均存在于修正版 run artifact。Ubuntu 24.04 只启动同一份 Ubuntu 22.04 AppImage。
- 五个平台 smoke JSON 均为 `launched: true`；分别记录 macOS arm64、macOS Intel、Windows x64、Ubuntu 22.04 x64 和 Ubuntu 24.04 x64 的真实启动路径及正式资产名。
- 修正版矩阵每个平台生成九张真实 Tauri `viewport/` 截图（1536×1024）及九张 `content/` 对比截图（1536×960）；每页 `.normalization.json`、`raw-snapshot.png` 和 `window-geometry.json` 均存在。Setup 逐页比较额外遵循 issue #5 已批准的无外层卡片基准；其余页面使用 Pencil 对应节点。比较记录见 `.artifacts/issue-16/visual-comparison.txt`，未发现未经批准的肉眼可见偏差。
- 本地最终验证：`corepack pnpm install --frozen-lockfile`、`corepack pnpm test:manifest`（3 passed）、`corepack pnpm typecheck`、`corepack pnpm exec vitest run`（184 passed）、`cargo test --manifest-path src-tauri/Cargo.toml`（204 passed）、`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 和 `git diff --check` 均通过；修正版 `MACOSX_DEPLOYMENT_TARGET=13.0 corepack pnpm tauri build -- --verbose` 产出 `.app`/`.dmg`，本机 packaged-app smoke `launched=true`，metadata 为 `OMP Switch`、最低 macOS `13.0`。
- Windows 客户端 gate 仍未满足：`windows-2022` 是 Windows Server 2022，不是 Windows 10 22H2 或 Windows 11 x64；当前没有可用的 Windows 客户端验收主机。不能用文档豁免，issue #16 不得关闭或发布 draft；取得客户端后必须重新运行 Windows 安装、启动、Rust/React seam 和九页 UI 证据。