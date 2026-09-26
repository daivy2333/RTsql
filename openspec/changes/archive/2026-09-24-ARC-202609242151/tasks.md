# ARC-202609242151 清理批次任务

> carrier 归档任务清单——全部步骤完成前不触碰源文档。

## Steps

- [x] T1 记录源文档 mtime（tasks 21:34 / improvements 20:40 / references 19:35 +0800）
- [x] T2 预检活跃 changes（`openspec/changes/` 仅 archive，`openspec list` 无活跃）
- [x] T3 生成 ARC ID 并创建 carrier 目录结构
- [x] T4 逐字抽取归档内容写入 `archive/improvements.md`（24 条）与 `archive/tasks-completed-roadmap.md`（6 块 395 行）
- [x] T5 写入 proposal 映射表（含排除项与侧效应声明）
- [ ] T6 验证 carrier（openspec validate）
- [ ] T7 OpenSpec 集成归档 carrier（--skip-specs）
- [ ] T8 复查源文档 mtime 未变
- [ ] T9 精准移除源条目：improvements 24 条、tasks 6 块
- [ ] T10 追加 arc 墓碑（improvements / tasks）
- [ ] T11 Artifact-Archive：m19/m21 分析移入 `.claude/analysis/archive/`，R06/R07 补路径注
- [ ] T12 Gate 2 验证：`openspec validate --all`、结构完整性、arc 计数、归档路径确认
