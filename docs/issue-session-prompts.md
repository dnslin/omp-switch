# OMP Switch 工单新会话提示词

使用方式：完成一个工单并关闭后，执行 `/clear`，开启新会话，复制下一条已解除阻塞的提示词。每条提示词均以 GitHub 工单为实施范围；不得从本文件推断或缩减验收标准。
```

## #12 — 手动测试四种 Supported protocol

```text
使用 /implement 完整实施 issue://12。先确认其全部 GitHub blockers 已关闭。

读取 issue://1、issue://12、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。UI 使用 /ui-design-guided 与 /vercel-react-best-practices；网络和页面行为使用 /tdd；完成后运行 /code-review。

按 Rust application-service seam 与 React page seam 做 red→green 纵向切片。Rust 每次从已保存配置构造四种 Supported protocol 的固定最小请求；全应用单并发、可取消、有超时，结果与错误分类完全脱敏。测试 UI 必须保持 Overview 节点 chFfk 和 Provider Detail 节点 vz6sg 的 1:1 布局，并复用 .pen 共享状态/Dialog 组件；运行真实 Tauri 状态截图对比。

满足 issue://12 全部验收标准，使用本地 Mock HTTP 服务验证 URL、方法、body、认证、错误、取消和超时；运行类型检查、真实流程 smoke、最终完整测试和双轴 review；更新文档，提交并关闭 #12。
```

## #13 — 安全删除无跨文件引用的 Provider 和模型

```text
使用 /implement 完整实施 issue://13。先确认其全部 GitHub blockers 已关闭。

读取 issue://1、issue://13、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/adr/0001-preserve-unknown-configuration-by-path.md、docs/adr/0003-keep-provider-and-model-ids-stable.md、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。UI 使用 /ui-design-guided 与 /vercel-react-best-practices；测试使用 /tdd；完成后运行 /code-review。

按两个既定 seam 做 red→green 纵向切片。删除前扫描受支持 Model role 与全部其他配置路径：无引用时执行单文件 Safe structured edit，非受管引用阻止删除，受支持角色引用明确移交跨文件事务流程。确认界面必须复用 .pen `Dialog / Confirm` 的尺寸、间距、阴影、按钮顺序和层级；分别截图验证模型删除、Provider 删除和阻止状态。

满足 issue://13 全部验收标准，覆盖引用扫描边界与原文件安全；运行针对性测试、类型检查、真实流程 smoke、最终完整测试和双轴 review；更新文档，提交并关闭 #13。
```

## #14 — 通过 Configuration transaction 删除受角色引用对象

```text
使用 /implement 完整实施 issue://14。先确认其 GitHub blockers 已关闭。

读取 issue://1、issue://14、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/adr/0001-preserve-unknown-configuration-by-path.md、docs/adr/0002-recover-cross-file-configuration-transactions.md、docs/adr/0003-keep-provider-and-model-ids-stable.md、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。UI 使用 /ui-design-guided 与 /vercel-react-best-practices；测试使用 /tdd；完成后运行 /code-review。

按两个既定 seam 做 red→green 纵向切片。实现双文件锁定/重读/Hash、共享备份、全部临时文件验证、持久事务清单、依次替换与确定性启动恢复；任何非完整最终状态都先保存现场并整体恢复，禁止部分恢复。确认和恢复状态复用 .pen Dialog、状态、卡片与页面视觉；运行真实 Tauri 状态截图核验。

满足 issue://14 全部验收标准，在关键故障点注入崩溃并证明完整提交清理或整体恢复；运行针对性测试、类型检查、恢复 smoke、最终完整测试和双轴 review；更新文档，提交并关闭 #14。
```

## #15 — 完成设置、OMP 切换与全局桌面交互

```text
使用 /implement 完整实施 issue://15。先确认其全部 GitHub blockers 已关闭。

读取 issue://1、issue://15、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/adr/0004-use-omp-config-path-as-authority.md、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。UI 使用 /ui-design-guided 与 /vercel-react-best-practices；行为测试使用 /tdd；完成后运行 /code-review。

按两个既定 seam 做 red→green 纵向切片。完成受限应用设置、目录入口、安全 OMP 切换、未保存确认、快捷键、焦点、通知和 reduced motion；新 OMP 验证失败或取消时保留当前可用选择。Settings 必须 1:1 还原 Pencil 节点 W7copJ；切换和确认复用批准 Dialog。运行真实 Tauri 设置页与关键 Dialog，在 1536×1024 截图对比。

满足 issue://15 全部验收标准，运行页面测试、类型检查、原生选择器/目录打开 smoke、最终完整测试和双轴 review；更新文档，提交并关闭 #15。
```

## #16 — 打包并通过 MVP 三平台发布验收

```text
使用 /implement 完整实施 issue://16。先确认其全部 GitHub blockers 已关闭。

读取 issue://1、issue://16、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/adr/ 下全部 ADR、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。发布验证使用 /tdd 中既定的高层 seam；UI 核验使用 /ui-design-guided；React 发布质量使用 /vercel-react-best-practices；完成后以实施前固定点运行 /code-review。

不要新增产品功能。完成 bundled manifest 构建校验、正式安装包和支持矩阵验收。逐平台运行真实应用，覆盖 OMP 检测、路径/权限/链接、配置 CRUD、四协议、Hash 冲突、单文件失败、Configuration transaction 恢复、备份隔离和秘密保护。对 Setup Success、Overview、Providers List、Provider Detail、Roles Dirty、Settings、Provider 创建两步和 Model Create Sheet 使用 1536×1024 截图，与 Pencil 对应节点/状态逐页比较；任何未经批准的肉眼可见偏差都是发布阻断项。

满足 issue://16 和 issue://1 的全部发布标准，运行 Rust/React 全套测试、类型检查、正式构建与每个平台真实验收，完成双轴 review；记录可复核证据，更新发布文档，提交并关闭 #16。不得用文档豁免测试或平台失败。
```
