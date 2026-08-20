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
- Windows x64：在 `windows-2022` 原生 runner 构建、启动并执行 Rust/React seam。
- Ubuntu 22.04/24.04 x64：Ubuntu 22.04 构建并执行 Rust/React seam；Ubuntu 24.04 下载并启动同一份 Ubuntu 22.04 `.AppImage`，smoke JSON 记录正式资产名。
- 每个平台的 runner 日志、bundle 和 smoke JSON 作为 workflow artifacts 保存 14 天。

原生应用窗口标题由 `src-tauri/tauri.conf.json` 的 Tauri window `title` 提供；页面不绘制第二套标题栏。UI 基准视口为 1536×1024，最小窗口为 1100×720；Setup 的最外层卡片例外遵循 issue #5 的已批准基准。


## 视觉证据

视觉权威是 Pencil 文件的以下节点：

- `R6EUPs`：Setup Success
- `chFfk`：Overview
- `H6IsaW`：Providers List
- `vz6sg`：Provider Detail
- `i5xFP`：Roles Dirty
- `W7copJ`：Settings
- `N3OTR` / `r67daM`：Foundations / Components

Provider 创建两步和 Model Create Sheet 使用对应 `.pen` 状态；issue #16 的原生截图分别位于 `.artifacts/issue-16/provider-create-step1-tauri-1536x961.png`、`.artifacts/issue-16/provider-create-step2-tauri-1536x961.png` 和 `.artifacts/issue-16/model-create-sheet-tauri-1536x961.png`，Retina 原件以 `*-tauri-retina.png` 保留；逐页比较记录位于 `.artifacts/issue-16/visual-comparison.txt`。未批准的肉眼可见差异不能通过文档豁免。

## 发布命令

```bash
gh workflow run platform-release.yml --ref <release-branch>
gh run list --workflow platform-release.yml
gh run watch <run-id>
```

验证所有 matrix job、bundle artifact、smoke JSON、Rust/React 测试和 manifest 校验均成功后，才可将 draft release 提交人工发布。