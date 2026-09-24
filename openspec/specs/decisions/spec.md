## Purpose

记录 RTsql 项目的重要架构决策（ADR）。**2026-09-24 起本域退役**（用户指令）：现行 `CLAUDE.md` 信息路由不再设决策域——长期选择及其理由写入 change 的 `design.md` 随 change 归档；历史条目 D01-D12 全文保存于清理 carrier，经下方 arc 指引定位。

## Requirements

### Requirement: 历史决策编号可解析

本域不再接受新条目；历史 Dxx 编号 SHALL 经 arc 指引定位 carrier 映射表解析。

#### Scenario: 解析历史决策编号

- **WHEN** 活跃文档引用历史 Dxx 编号（如 D09、D12）
- **THEN** 经本文件底部 arc 指引定位清理 carrier 的 proposal 映射表与 `archive/decisions.md` 取回原文

---

<!-- arc: ARC-202609241843b --> 12 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609241843b/proposal.md（本域退役：长期选择理由改由 change design.md 承载）
