# proposal: ARC-202609241843a improvements 预批准条目归档（I018/I027）

## Why

用户 2026-09-12 批准 I018/I027 归档，条目自注「已裁定归档（待 openspec-archivist 物理归档）」至今未执行；2026-09-24 用户触发 openspec-archivist 全量审计并批准实施（Gate 1 豁免），本 carrier 承接该预批准批次。工作树基线：HEAD `5a64b56` + 未提交 docs 改动（tasks K37 行、improvements I062-I066，本批次不触碰）；`openspec validate --all` 40 passed / 0 failed（清理前新鲜复跑）。

## 映射表

### Archive（进入本 carrier）

| 原编号 | 源文档 | 动作 | 归档位置 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|---|---|
| I018 | `openspec/specs/improvements/spec.md` | Archive | `archive/improvements.md`（本 change） | 用户 2026-09-12 批准归档：单层关联子查询缓存已随 MS09-T04 实施落地（spec `correlated-subquery-cache`），多层嵌套无当前需求 | 全仓扫描仅 legacy 迁移映射（`.claude/legacy/`COVERAGE.md O018→I018）；编号保留可解析 | 用户提出多层关联子查询需求时，Maintainer 从本映射表定位，取 `archive/improvements.md` 原文插回 `openspec/specs/improvements/spec.md` |
| I027 | 同上 | Archive | 同上 | 用户 2026-09-12 批准归档：复杂度高且既有测试未证明扫描争用（风险收益比与 tasks D-candidates 同级裁定） | 全仓扫描仅 legacy COVERAGE.md O027→I027 | 同上（需求重启时按 I027 原方案重启评估） |

## 本次明确排除的条目

- I021（同批 2026-09-09 Stale-Warn 项）：已随 2026-09-14 MS08 剥离退还流程 planned 化，非归档对象，本批不动。
- improvements 其余 43 条：2026-09-24 审计判定 Keep（时效内登记/用户方向/promoted 记录），见会话审计报告。
- M07/M16 与 decisions/knowledge 退役：独立批次，见 carrier ARC-202609241843b。

## 侧效应声明

本 carrier 归档后，源文档 `openspec/specs/improvements/spec.md` 精准移除 I018/I027 两条目并追加 `<!-- arc: ARC-202609241843a -->` 墓碑；该文件另有用户未提交改动（I062-I066），与本批次编辑位置不相交。
