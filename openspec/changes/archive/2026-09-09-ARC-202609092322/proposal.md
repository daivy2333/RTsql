# proposal: ARC-202609092322 文档体系生命周期清理（improvements 陈旧条目 + R05 + 旧工作流操作文件）

## Why

用户显式触发 openspec-archivist 清理（2026-09-09）。依据本会话 Explorer 两轮调查（代码现场核实 + 文档体系盘点）与用户逐项批准（Gate 1，2026-09-09）执行。工作树基线：`a5b0a5f` + 未提交 MS10-T05 实施；`openspec validate --all` 19 passed / 0 failed（清理前新鲜复跑）。

## 映射表

### Archive（进入本 carrier）

| 原编号 | 源文档 | 动作 | 归档位置 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|---|---|
| I009 | `openspec/specs/improvements/spec.md` | Archive | `specs/improvements/spec.md`（本 change） | 被代码现实替代：RowLockTable 已改为 `RwLock<HashMap<RowId, Arc<Mutex<()>>>>` 且整类无调用者（死代码，MVCC 取代行锁）；条目描述的瓶颈对象不存在 | 全仓扫描无外部引用（仅 improvements spec 自身） | Maintainer 从本 proposal 映射表定位原编号，从 carrier spec 取回全文插回 |
| I010 | 同上 | Archive | 同上 | 被替代：Group Commit 已落地（`wal/buffer.rs` append_commit_and_wait + `write_batch` 单次 fsync），"每事务单独 fsync"前提不成立（MS00 基线即有） | 同上 | 同上 |
| I011 | 同上 | Archive | 同上 | 部分被替代：WALBuffer capacity=100 触发同步刷盘已构成隐式背压（`buffer.rs:62-76`）；无显式水位机制但原问题不再以登记形态存在 | 同上 | 同上 |
| I013 | 同上 | Archive | 同上 | 判定规则"change 已归档：Archive"：已由 MS08-T01 实施落地（`file_storage.rs:101-121` `FileExt::read_exact_at/write_all_at`），change `2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch` 已归档 | 同上 | 同上 |
| I019 | 同上 | Archive | 同上 | 被用户决策降级（2026-09-06，tasks MS09 non-goals 记录）：非交互 CLI 形态下无消费者，server 保留为库能力 | 同上 | 同上 |
| I022 | 同上 | Archive | 同上 | 判定规则"change 已归档：Archive"：已由 MS07-T01 实施落地（`catalog.rs` + `open_or_init`），change `2026-08-26-2026-08-26-ms07-t01-schema-persistence` 已归档 | 同上 | 同上 |
| I023 | 同上 | Archive | 同上 | 判定规则"change 已归档：Archive"：已由 MS08-T02 实施落地（`with_prefetch` 默认关），change 同 I013 | 同上 | 同上 |
| R05 | `openspec/specs/references/spec.md` | Archive | `specs/references/spec.md`（本 change） | 失效快照：2026-06-04 测试统计（475 tests）与现状（704）严重脱节；数值型现场数据职责由 SNAPSHOT 仓库现场承载；交叉引用扫描无命中；MEDIUM 置信度经用户批准归档 | 全仓扫描无外部引用 | 同上 |

### Delete（无 carrier 原文，映射留档）

| 目标 | 动作 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|
| `.claude/commands/opsx/`（5 文件） | Delete | 用户显式确认（2026-09-09 Gate 1："可以删除"）；旧 OPSX experimental 工作流命令包装，已被 .agents/skills/ 的 10 个 openspec-* skill 替代 | 引用仅存在于同批删除的 .claude/skills 旧 skill 与不可变归档 change（历史提及，不处理） | 不可逆删除；如需恢复按 openspec-init 重生成 |
| `.claude/skills/openspec-{apply-change,archive-change,explore,propose,sync-specs}/` | Delete | 用户批准。旧 OPSX workflow skill，引用被删的 `/opsx:*` 命令；当前会话技能清单不含（未被加载）；由 .agents/skills/ 全集替代 | AGENTS.md 加载入口一节提及 ".claude/skills/"——已向用户提交 SUGGEST-REVIEW（AGENTS.md 归用户维护，本批次不改） | 不可逆删除；如需恢复按 openspec-init 重生成 |

### Stale-Warn（原地标记，不移动）

| 原编号 | 源文档 | 理由 |
|---|---|---|
| I018 / I021 / I027 | `openspec/specs/improvements/spec.md` | 仍成立的真缺口（代码事实已核实）但 6 月登记至今无路线图归属（>90 天）；标记 30 天内确认、更新或归档，强制后续规划决断 |

## 本次明确排除的条目

- m19-datascan-path.md / m21-page-visibility-incomplete.md：**Keep**（新鲜交叉引用发现 R18 `usability-gap-cli-form.md` 以相对链接活跃引用，有引用不满足 Delete/Artifact 条件——分析阶段改判）。
- 19 个 change 归档 carrier：**Keep**（CLAUDE/tasks 规则：归档 carrier 不可变；旧体系归档只允许完整 Archive）。
- `.claude/legacy/2026-08-25-openspec-init-migration/`：**Keep**（不可变 migration carrier）。
- `.claude/runbooks/ms08-bench-comparison.md`（R17）：**Keep**（active，MS08-T03-T06 待实施直接需要）。
- I012/I014/I015/I016/I017/I020/I024/I025/I026/I028/I029/I030/I031-I040：**Keep**（路线图覆盖、D-candidates 跟踪、长期方向或新鲜条目）。
- R01-R04、R06-R20：**Keep**（本批次范围外；R01 缺 clap/csv 交 docs-maintainer）。

## 移交清单（不在本批次执行）

- docs-maintainer：R01 补 clap/csv；I032/I034/I039 描述按新证据修订；登记 tx_id 计数器重启重叠新 I 候选（database.rs:60-66 丢弃 `_max_tx_id`）；10 个归档 change 的 R 索引缺口裁定（回填 R 条目或废弃"已归档 Change 索引"段约定）。
- milestone-planner：MS08-T04（目标 RowLockTable 为死代码）与 MS08-T06（fsync 合并前提已被 Group Commit 满足）重评估。
- 用户：AGENTS.md 加载入口行（提及 .claude/skills/）修订。

## 验证基线

- 清理前：`openspec validate --all` 19 passed / 0 failed（2026-09-09 23:22 前后新鲜复跑）。
- 源文档 mtime 记录：improvements/spec.md 2026-09-09 21:52:07；references/spec.md 2026-09-09 14:06:51（执行前再次核对）。
