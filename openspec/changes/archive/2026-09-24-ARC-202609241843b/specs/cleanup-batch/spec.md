## ADDED Requirements

### Requirement: 清理批次可审计

本 change 是 openspec-archivist 清理批次 ARC-202609241843b 的归档载体（--skip-specs 归档，不应用于主 specs）。归档条目全文 SHALL 保存在 change 的 `archive/` 子目录，恢复映射 SHALL 保存在 `proposal.md`。

#### Scenario: 恢复归档条目

- **WHEN** Maintainer 需要恢复被归档的 M07/M16、D01-D12 或 K01-K38
- **THEN** 从 proposal 映射表定位原编号与源文档，从 `archive/project-model.md` / `archive/decisions.md` / `archive/knowledge.md` 取回完整原文插回
