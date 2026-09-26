## ADDED Requirements

### Requirement: 清理批次可审计

本 change 是 openspec-archivist 清理批次 ARC-202609242151 的归档载体（--skip-specs 归档，不应用于主 specs）。归档条目全文 SHALL 保存在 change 的 `archive/` 子目录，恢复映射 SHALL 保存在 `proposal.md`。

#### Scenario: 恢复归档的 improvements 条目

- **WHEN** Maintainer 需要恢复被归档的 24 条 improvements（I014/I017/I032-I048 已实施组、I042/I059/I061-I072 已排期组）
- **THEN** 从 proposal 映射表定位原编号，从 `archive/improvements.md` 取回完整原文插回 `openspec/specs/improvements/spec.md` 对应 Phase 位置

#### Scenario: 恢复 tasks 已完成路线

- **WHEN** Maintainer 需要恢复 MS00-MS17 段、历史流程图、最近完成表或规划理念/执行顺序段
- **THEN** 从 proposal 映射表定位块名，从 `archive/tasks-completed-roadmap.md` 取回原文插回 `.claude/docs/tasks.md` 原位置

#### Scenario: 恢复 Artifact 归档的分析文档

- **WHEN** 需要恢复 `m19-datascan-path.md` 或 `m21-page-visibility-incomplete.md`
- **THEN** 从 `.claude/analysis/archive/` 取回文件并按 R06/R07 路径注恢复
