# OMP Switch 工单新会话提示词

使用方式：完成一个工单并关闭后，执行 `/clear`，开启新会话，复制下一条已解除阻塞的提示词。每条提示词均以 GitHub 工单为实施范围；不得从本文件推断或缩减验收标准。
```


## #16 — 打包并通过 MVP 三平台发布验收

```text
使用 /implement 完整实施 issue://16。先确认其全部 GitHub blockers 已关闭。

读取 issue://1、issue://16、CONTEXT.md、docs/mvp.md、docs/prd.md、docs/flow.md、docs/adr/ 下全部 ADR、docs/agents/issue-tracker.md，以及 designs/omp-switch.pen。发布验证使用 /tdd 中既定的高层 seam；UI 核验使用 /ui-design-guided；React 发布质量使用 /vercel-react-best-practices；完成后以实施前固定点运行 /code-review。

不要新增产品功能。完成 bundled manifest 构建校验、正式安装包和支持矩阵验收。逐平台运行真实应用，覆盖 OMP 检测、路径/权限/链接、配置 CRUD、四协议、Hash 冲突、单文件失败、Configuration transaction 恢复、备份隔离和秘密保护。对 Setup Success、Overview、Providers List、Provider Detail、Roles Dirty、Settings、Provider 创建两步和 Model Create Sheet 使用 1536×1024 截图，与 Pencil 对应节点/状态逐页比较；任何未经批准的肉眼可见偏差都是发布阻断项。

满足 issue://16 和 issue://1 的全部发布标准，运行 Rust/React 全套测试、类型检查、正式构建与每个平台真实验收，完成双轴 review；记录可复核证据，更新发布文档，提交并关闭 #16。不得用文档豁免测试或平台失败。
```
