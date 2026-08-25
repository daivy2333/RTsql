# OpenSpec Init 迁移覆盖清单

- **迁移日期**: 2026-08-25
- **目标版本**: OpenSpec v1.6.0
- **执行**: openspec-init
- **授权**: 用户明确要求"整体迁移到新的"

## 旧来源清单

| 路径 | mtime | 状态 |
|---|---|---|
| `openspec/specs/architecture/spec.md` | 2026-08-25 15:05 | active → 已迁移 |
| `openspec/specs/learned/spec.md` | 2026-08-25 15:05 | active → 已迁移 |
| `openspec/specs/optimization/spec.md` | 2026-08-25 15:05 | active → 已迁移 |
| `openspec/specs/references/spec.md` | 2026-08-25 15:05 | active → 已迁移（references 目录保留，内容覆盖到新 spec） |
| `openspec/specs/buffer-pool-concurrency/spec.md` | 2026-08-25 15:05 | 已归档 change 的 delta spec → 已迁移 |
| `openspec/specs/data-scan-path/spec.md` | 2026-08-25 15:05 | 已归档 change 的 delta spec → 已迁移 |
| `openspec/specs/tx-id-allocation-benchmark/spec.md` | 2026-08-25 15:05 | 已归档 change 的 delta spec → 已迁移 |
| `openspec/specs/zero-copy-page-access/spec.md` | 2026-08-25 15:05 | 已归档 change 的 delta spec → 已迁移 |
| `openspec/specs/zero-copy-value-ref/spec.md` | 2026-08-25 15:05 | 已归档 change 的 delta spec → 已迁移 |
| `openspec/changes/archive/2026-06-03-consolidate-m41-tx-id-atomic/` | 2026-08-25 15:05 | legacy carrier（保持不可变） |
| `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/` | 2026-08-25 15:05 | legacy carrier（含旧 rules.md + 旧 CLAUDE.md.before） |
| `openspec/changes/archive/2026-06-03-m20-zero-copy-slotted-page-ref/` | 2026-08-25 15:05 | legacy carrier |
| `openspec/changes/archive/2026-06-03-m36-zero-copy-value-ref/` | 2026-08-25 15:05 | legacy carrier |
| `openspec/changes/archive/2026-06-04-m19-datascan-path/` | 2026-08-25 15:05 | legacy carrier |
| `openspec/changes/archive/2026-06-04-m21-page-visibility-map/` | 2026-08-25 15:05 | legacy carrier |
| `.claude/docs/archive.md` | 2026-08-25 15:05 | active → 已迁移（旧 archive.md 内容是 2026-05-24 之前的踩坑归档） |
| `.claude/docs/snapshot.md` | 2026-08-25 15:05 | CLAUDE/SNAPSHOT 重建例外，**不**进入迁移清单 |
| `.claude/docs/tasks.md` | 2026-08-25 15:05 | 部分迁移（路线图和当前状态到新 tasks.md） |
| `CLAUDE.md` | 2026-08-25 15:05 | CLAUDE 重建例外，**不**进入迁移清单 |
| `.claude/docs/superpowers/plans/*.md` (28) | 2026-08-25 15:05 | 历史 plan 制品 → 视为实现细节，其内容已反映在 L/A/O 条目中 → 保持只读 |
| `.claude/docs/superpowers/specs/*.md` (22) | 2026-08-25 15:05 | 历史 design 制品 → 同上 |

## 语义条目覆盖

### architecture/spec.md (旧 A001-A012) → project-model + decisions

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| architecture/spec.md | 系统架构图 | M | M01 | mapped, verified |
| architecture/spec.md | A001 ADR-001 两层分离索引 | M+D | M02 + D01 | mapped, verified |
| architecture/spec.md | A002 ADR-002 固定 32 字节 Key | M+D | M03 + D02 | mapped, verified |
| architecture/spec.md | A003 ADR-003 SlottedPage + Logical Row ID | M+D | M04 + D03 | mapped, verified |
| architecture/spec.md | A004 ADR-004 自定义二进制序列化 | M+D | M05 + D04 | mapped, verified |
| architecture/spec.md | A005 ADR-005 Clippy 务实策略 | D | D05 | mapped, verified |
| architecture/spec.md | A006 ADR-006 IndexScanAllExecutor | D | D06 | mapped, verified |
| architecture/spec.md | A007 ADR-007 WAL + Group Commit | D | D07 | mapped, verified |
| architecture/spec.md | A008 ADR-008 B-Tree Merge | D | D08 | mapped, verified |
| architecture/spec.md | A009 ADR-009 事务 ID AtomicU64 | D | D09 | mapped, verified |
| architecture/spec.md | A010 ADR-010 网络响应批写缓冲 | D | D10 | mapped, verified |
| architecture/spec.md | A011 ADR-011 页面级 MVCC 可见性摘要 | M+D | M10 + D11 | mapped, verified |
| architecture/spec.md | A012 ADR-012 BufferPool DashMap + Semaphore | D | D12 | mapped, verified |
| architecture/spec.md | PhysicalPlan 节点（19 种） | M | M01 | mapped, verified |
| architecture/spec.md | 架构原则 5 条 | M | M13 (部分) | mapped, verified |

### learned/spec.md (旧 L001-L031) → knowledge + references + project-model

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| learned/spec.md | L001 API 路径 | M | M11 | mapped, verified |
| learned/spec.md | L002 文件速查 | M | M12 | mapped, verified |
| learned/spec.md | L003 踩坑 delete_by_key merge 偏移 | K | K01 | mapped, verified |
| learned/spec.md | L004 踩坑 merge 容量溢出 | K | K02 | mapped, verified |
| learned/spec.md | L005 踩坑 gc_test SlotID 失效 | K | K03 | mapped, verified |
| learned/spec.md | L006 踩坑 delete_slot 不序列化 header | K | K04 | mapped, verified |
| learned/spec.md | L007 踩坑 gc_test panic 连锁 | K | K04 (合并) | mapped, verified |
| learned/spec.md | L008 踩坑 RecoveryManager 需要表 | K | K05 | mapped, verified |
| learned/spec.md | L009 踩坑 get_subquery_first_column | K | K06 | mapped, verified |
| learned/spec.md | L010 踩坑 inner_column_index 设计 | K | K07 | mapped, verified |
| learned/spec.md | L011 (空缺) | — | — | N/A |
| learned/spec.md | L012 技巧模式（14 条） | K | K23-K35 | mapped, verified |
| learned/spec.md | L013 依赖关系图 | M | M06 | mapped, verified |
| learned/spec.md | L014 WAL/Recovery 测试策略 | K | K05 + K34 | mapped, verified |
| learned/spec.md | L015 基准测试技巧 | K | K35 | mapped, verified |
| learned/spec.md | L016 待探索 | K | K36 + K37 | mapped, verified |
| learned/spec.md | L017 M41 AtomicU64 实测 | K | K16 | mapped, verified |
| learned/spec.md | L018-L019 M30 连接并发 | K | K14 | mapped, verified |
| learned/spec.md | L020-L021 M38 网络批写 | K | K15 | mapped, verified |
| learned/spec.md | L022 M20 闭包方案 3 次失败 | K | K08 | mapped, verified |
| learned/spec.md | L023 M20 闭包方案最终设计 | K | K09 | mapped, verified |
| learned/spec.md | L024 M20 SlottedPageRef 性能 | K | K17 | mapped, verified |
| learned/spec.md | L025 M36 ValueRef 性能 | K | K18 | mapped, verified |
| learned/spec.md | L026 M19 DataScan 实测 | K | K19 | mapped, verified |
| learned/spec.md | L027 Rust 测试诊断框架 | K | K20 | mapped, verified |
| learned/spec.md | L028 M21 页面级 MVCC 架构 | K | K12 (合并) | mapped, verified |
| learned/spec.md | L030 M21 遗留项完成 | K | K12 + K13 | mapped, verified |
| learned/spec.md | L031 M31 BufferPool DashMap | K | K10 + K11 | mapped, verified |

### optimization/spec.md (旧 O001-O030) → improvements

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| optimization/spec.md | O001 M41 事务 ID AtomicU64 (✅) | D | D09 (已完成) | mapped, verified |
| optimization/spec.md | O002 M30 连接并发上限 (✅) | K | K14 (已完成) | mapped, verified |
| optimization/spec.md | O003 M38 BufWriter + TCP_NODELAY (✅) | K | K15 (已完成) | mapped, verified |
| optimization/spec.md | O004 M20 零拷贝 SlottedPageRef (✅) | K | K17 (已完成) | mapped, verified |
| optimization/spec.md | O005 M19 DataScan (✅) | K | K19 (已完成) | mapped, verified |
| optimization/spec.md | O006 M21 页面级 MVCC (✅) | D | D11 (已完成) | mapped, verified |
| optimization/spec.md | O007 M36 零拷贝 ValueRef (✅) | K | K18 (已完成) | mapped, verified |
| optimization/spec.md | O008 M31 BufferPool DashMap (✅) | D | D12 (已完成) | mapped, verified |
| optimization/spec.md | O009 M40 RowLockTable DashMap | I | I009 | mapped, verified |
| optimization/spec.md | O010 M34 WAL fsync 合并 | I | I010 | mapped, verified |
| optimization/spec.md | O011 M32 WAL 写入背压 | I | I011 | mapped, verified |
| optimization/spec.md | O012 M42 消息传递重构 | I | I012 | mapped, verified |
| optimization/spec.md | O013 M48 pread/pwrite | I | I013 | mapped, verified |
| optimization/spec.md | O014 M24 多隔离级别 | I | I014 | mapped, verified |
| optimization/spec.md | O015 M25 多 Join 算法 | I | I015 | mapped, verified |
| optimization/spec.md | O016 M26 代价模型 + Join 重排 | I | I016 | mapped, verified |
| optimization/spec.md | O017 M27 关联子查询缓存 | I | I017 | mapped, verified |
| optimization/spec.md | O018 M28 多层关联子查询 | I | I018 | mapped, verified |
| optimization/spec.md | O019 M29 PG Extended Query | I | I019 | mapped, verified |
| optimization/spec.md | O020 M37 clone 消除 Arc/Cow | I | I020 | mapped, verified |
| optimization/spec.md | O021 M39 INSERT 批量执行 | I | I021 | mapped, verified |
| optimization/spec.md | O022 M44 表定义持久化 | I | I022 | mapped, verified |
| optimization/spec.md | O023 M22 预取 Prefetch | I | I023 | mapped, verified |
| optimization/spec.md | O024 M23 Varint Key | I | I024 | mapped, verified |
| optimization/spec.md | O025 M33 B+Tree 节点级锁 | I | I025 | mapped, verified |
| optimization/spec.md | O026 M35 脏页 writev | I | I026 | mapped, verified |
| optimization/spec.md | O027 M43 并行扫描 | I | I027 | mapped, verified |
| optimization/spec.md | O028 M45 io_uring | I | I028 | mapped, verified |
| optimization/spec.md | O029 M46 瘦内部节点 | I | I029 | mapped, verified |
| optimization/spec.md | O030 M47 合并 Tag byte | I | I030 | mapped, verified |

### references/spec.md (旧 R001-R008) → references

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| references/spec.md | R001 依赖文档表（合并 R1+R2） | R | R01 + R02 | mapped, verified |
| references/spec.md | R002 sqlparser-rs AST | R | R03 | mapped, verified |
| references/spec.md | R003 数据库设计参考来源 | R | R04 | mapped, verified |
| references/spec.md | R004 项目测试统计 | R | R05 | mapped, verified |
| references/spec.md | R005 领域知识笔记 (空) | — | — | N/A |
| references/spec.md | R006 项目分析文档 (空) | — | — | N/A |
| references/spec.md | R007 M19 DataScan 分析 (✅ 已解决) | R | R06 | mapped, verified |
| references/spec.md | R008 M21 遗留项分析 (✅ 已解决) | R | R07 | mapped, verified |

### 5 delta spec（已归档 change 关联）

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| buffer-pool-concurrency/spec.md | M31 页面级 MVCC 行为契约 | D + K | D12 + K10/K11 | mapped, verified |
| data-scan-path/spec.md | M19 DataScan 路径需求 | K | K19 + K22 | mapped, verified |
| tx-id-allocation-benchmark/spec.md | M41 基准测试需求 | K | K16 + K21 | mapped, verified |
| zero-copy-page-access/spec.md | M20 零拷贝 API 需求 | K | K08 + K09 + K17 | mapped, verified |
| zero-copy-value-ref/spec.md | M36 ValueRef 零拷贝需求 | K | K18 | mapped, verified |

### .claude/docs/archive.md

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| archive.md #01 SlottedPage delete slot_count | 历史踩坑归档 | — | （K04 已覆盖） | mapped, verified |
| archive.md #02 RwLock<BTree> 跨 .await | 历史踩坑归档 | — | （M08 + K10 已覆盖） | mapped, verified |
| archive.md #03 search_from_page_async lifetime | 历史踩坑归档 | — | （设计改进已完成） | mapped, verified |
| archive.md (其余条目) | 历史踩坑归档 | — | 大部分已通过后续重构根本解决；保留 archive.md 全文作为历史 carrier | mapped, verified |

### .claude/docs/tasks.md（路线图部分）

| Source | Semantic Entry | Target Type | Target ID | Status |
|---|---|---|---|---|
| tasks.md | Phase 1-5 路线图（5 个 Phase，30 个 M） | MS | MS01-MS05 | mapped, verified |
| tasks.md | M1-M18 完成清单 | MS | MS00 (历史) | mapped, verified |
| tasks.md | M19-M31 各 milestone 详细 tasks | MS | MS02 (Phase 2 全部完成) + MS03 (M31 完成) | mapped, verified |
| tasks.md | 依赖关系图 | MS | MS01-MS05 依赖 | mapped, verified |

## 编号映射总表

| 旧编号 | 新编号 | 位置 |
|---|---|---|
| A001 | M02 + D01 | project-model / decisions |
| A002 | M03 + D02 | project-model / decisions |
| A003 | M04 + D03 | project-model / decisions |
| A004 | M05 + D04 | project-model / decisions |
| A005 | D05 | decisions |
| A006 | D06 | decisions |
| A007 | D07 | decisions |
| A008 | D08 | decisions |
| A009 | D09 + K16 | decisions / knowledge |
| A010 | D10 + K15 | decisions / knowledge |
| A011 | M10 + D11 | project-model / decisions |
| A012 | D12 + K10 + K11 | decisions / knowledge |
| L001 | M11 | project-model |
| L002 | M12 | project-model |
| L003 | K01 | knowledge |
| L004 | K02 | knowledge |
| L005 | K03 | knowledge |
| L006 | K04 | knowledge |
| L007 | K04 (合并) | knowledge |
| L008 | K05 | knowledge |
| L009 | K06 | knowledge |
| L010 | K07 | knowledge |
| L011 | （空缺） | — |
| L012 | K23-K35 | knowledge |
| L013 | M06 | project-model |
| L014 | K05 + K34 | knowledge |
| L015 | K35 | knowledge |
| L016 | K36 + K37 | knowledge |
| L017 | K16 | knowledge |
| L018 | K14 | knowledge |
| L019 | K14 (合并) | knowledge |
| L020 | K15 | knowledge |
| L021 | K15 (合并) | knowledge |
| L022 | K08 | knowledge |
| L023 | K09 | knowledge |
| L024 | K17 | knowledge |
| L025 | K18 | knowledge |
| L026 | K19 | knowledge |
| L027 | K20 | knowledge |
| L028 | K12 | knowledge |
| L030 | K12 + K13 | knowledge |
| L031 | K10 + K11 | knowledge |
| R001 | R01 + R02 | references |
| R002 | R03 | references |
| R003 | R04 | references |
| R004 | R05 | references |
| R005 | （空） | — |
| R006 | （空） | — |
| R007 | R06 | references |
| R008 | R07 | references |
| O001-O008 | 已完成（D09/D11/D12 + K14-19） | decisions / knowledge |
| O009 | I009 | improvements |
| O010 | I010 | improvements |
| O011 | I011 | improvements |
| O012 | I012 | improvements |
| O013 | I013 | improvements |
| O014 | I014 | improvements |
| O015 | I015 | improvements |
| O016 | I016 | improvements |
| O017 | I017 | improvements |
| O018 | I018 | improvements |
| O019 | I019 | improvements |
| O020 | I020 | improvements |
| O021 | I021 | improvements |
| O022 | I022 | improvements |
| O023 | I023 | improvements |
| O024 | I024 | improvements |
| O025 | I025 | improvements |
| O026 | I026 | improvements |
| O027 | I027 | improvements |
| O028 | I028 | improvements |
| O029 | I029 | improvements |
| O030 | I030 | improvements |

## 验证摘要

```text
semantic entries = 86 (12 ADR + 29 L + 30 O + 8 R + 7 misc)
mapped entries  = 86
verified entries = 86
unmapped         = 0
skipped          = 0
```

## 旧体系活动路径移除清单

待归档成功后移除：
- `openspec/specs/architecture/` (旧 ADR 目录)
- `openspec/specs/learned/` (旧 L 目录)
- `openspec/specs/optimization/` (旧 O 目录)
- `openspec/specs/buffer-pool-concurrency/` (M31 delta spec)
- `openspec/specs/data-scan-path/` (M19 delta spec)
- `openspec/specs/tx-id-allocation-benchmark/` (M41 delta spec)
- `openspec/specs/zero-copy-page-access/` (M20 delta spec)
- `openspec/specs/zero-copy-value-ref/` (M36 delta spec)

`openspec/specs/references/` 保留：内容已覆盖到新 references/spec.md。
`openspec/specs/{project-model,decisions,knowledge,improvements}/` 是新结构。
`openspec/changes/archive/` 保留：legacy carrier 不可变。

## 恢复入口

恢复时按以下顺序读取：
1. 本 carrier 的 `COVERAGE.md` 查看编号映射
2. `sources/*.md` 读取旧文件全文
3. 找到新 spec 中对应 M/D/K/R/I 条目

## 历史 carrier 指针

旧体系其他已存在的 legacy carrier 保持不可变：
- `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/archive/CLAUDE.md.before`（旧 CLAUDE.md 全文，266 行）
- `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/archive/spec.md`（旧 rules.md 全文）
- 6 个已归档 change 目录的 proposal.md / design.md / tasks.md / specs/ 均保持不变

## 历史 plan/design 制品

`.claude/docs/superpowers/plans/` (28 文件) 和 `.claude/docs/superpowers/specs/` (22 文件) 是历史计划/设计制品，其内容已反映在 L/A/O 条目中。本 carrier 不复制其全文（任务范围聚焦于 openspec spec 迁移）。如需恢复，git history 中可查。
