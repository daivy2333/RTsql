# proposal: ARC-202609241843b 文档体系退役清理（project-model 陈旧模型条目 + decisions/knowledge 域退役与知识迁移）

## Why

用户 2026-09-24 触发 openspec-archivist 全量审计，Gate 1 批准实施并豁免 skill 调用边界（Runbook/Analysis/R 登记 Normally 归 Recorder/Explorer/Maintainer，本批为逐字迁移获用户明示授权）。核心事实：现行 `CLAUDE.md` 文档地图与信息路由均无 decisions/knowledge 域（长期选择理由已改路由 change design.md；已验证结论沉淀 analysis；操作流程入 Runbook），两 spec 为脱离现行体系的遗留物理文件且已积累陈旧内容（K12 所述 mark_deleted 机制已被 MS09 墓碑 slot 化取代等），用户指令「把对应知识迁移到现有的持久化产物承担」。project-model 的 M07/M16 经代码现场核验失效（M07 两阶段锁机制已被 DashMap 模型整体替换，`src/storage/buffer_pool.rs` 核验；M16 四条已知限制被 MS07-T01/MS09 推翻）。工作树基线：HEAD `5a64b56` + 未提交 docs 改动（I062-I066/K37 行，编辑位置不相交）；`openspec validate --all` 40 passed / 0 failed（清理前新鲜复跑）。

## 映射表

### Archive（进入本 carrier）

| 原编号 | 源文档 | 动作 | 归档位置 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|---|---|
| M07 | `openspec/specs/project-model/spec.md` | Archive | `archive/project-model.md` | 不变量所述两阶段锁加载已被 D12（DashMap + miss Sem + per-page loading locks）整体替换（代码核验）；状态行「active（…增强）」自相矛盾；现役模型由本批新增 M17 承担 | K24 Legacy 谱系行（编号保留可解析） | Maintainer 从本映射表定位，取 archive 原文插回 |
| M16 | 同上 | Archive | 同上 | 四条限制全部失效：表定义不持久化被 MS07-T01 推翻、仅 RR 被 MS09 推翻、文件大小数字与 M02 矛盾且被 README 对比板块取代、扫描条目为历史记录；现役限制权威在双语 README 与行为 specs | 全仓扫描无引用 | 同上 |
| D01-D04 | `openspec/specs/decisions/spec.md` | Archive | `archive/decisions.md` | 与 project-model M02/M03/M04/M05 内容重复（含代价/影响字段）；独特信息仅日期与替代方案 | improvements I024 Legacy 提及 D02（历史编号经墓碑可解析） | 同上 |
| D05-D08、D10 | 同上 | Archive | 同上 | 历史实现/过程决策，行为已由 specs 语料库、代码与测试锁定；D07 现役约束由 spec `wal-writer-handle-reuse` 承载 | 无活跃反向引用 | 同上 |
| D09 | 同上 | Archive | 同上 | 约束要义（AtomicU64 单调分配）已并入 project-model M06（本批 Merge）；实测数据与 K16 重复 | R08 关联决策 D09；M06 新增行注明 carrier 来源 | 同上 |
| D11 | 同上 | Archive | 同上 | 现行可见性模型权威在 M10 + spec `mvcc-tombstone-visibility`；子决策 4（mark_deleted）已被 MS09 墓碑 slot 化取代（条目内陈旧） | R13 关联决策 D11 | 同上 |
| D12 | 同上 | Archive | 同上 | 现役并发模型已升格 project-model M17（本批 Merge）；决策全文（含子决策/替代方案/代价）随 carrier 保留 | K10/K11 Legacy；I026 Legacy「D12 下游」 | 同上 |
| K01-K04、K06-K09 | `openspec/specs/knowledge/spec.md` | Archive | `archive/knowledge.md` | 现仍成立的踩坑根因/设计教训，按信息路由迁入 analysis R28（逐字）；carrier 保存原文 | R10-R13 关联知识字段；R28 | 同上 |
| K10-K11 | 同上 | Archive | 同上 | 当前约束（锁顺序/per-page lock 设计）已升格 project-model M17；全文随 carrier | M17 Legacy 行 | 同上 |
| K12-K13 | 同上 | Archive | 同上 | 陈旧：K12 机制被 MS09 墓碑 slot 化取代（权威 spec `mvcc-tombstone-visibility`）；K13 三条件被 ISS01/MS17-T02 哨兵语义修订；不迁移仅留档 | R07（已改写指向现行权威） | 同上 |
| K14-K19 | 同上 | Archive | 同上 | 现仍成立的模式与历史性能数据，迁入 analysis R28 | R10-R12 关联知识；K16 与 D09 数据重复 | 同上 |
| K20-K21、K34-K35 | 同上 | Archive | 同上 | 可复用方法论，迁入 runbook `test-bench-diagnosis.md`（R27）；K21 与 R17 互补 | R17 | 同上 |
| K22-K33 | 同上 | Archive | 同上 | 现仍成立的技巧模式，迁入 analysis R28 | M11/M13 覆盖其中 API 面 | 同上 |
| K36-K37 | 同上 | Archive | 同上 | 待探索指针已由 improvements I028/I066 承担（无独有内容）；K37 与 I066 合并冗余 | tasks 长期方向（已改写指 I028/I066）；I028 Legacy K36 | 同上 |
| K38 | 同上 | Archive | 同上 | 现行引擎不变量已升格 project-model M18（本批 Merge）；全文随 carrier | I031「实施前必读本条」改读 M18；M18 Legacy 行 | 同上 |

### Merge（内容迁入现存产物，本批已执行）

| 目标 | 来源 | 内容 |
|---|---|---|
| project-model M17（新增） | D12+K10+K11 | BufferPool 现役并发模型（DashMap/miss Sem/per-page locks/锁顺序/collect-then-write） |
| project-model M18（新增） | K38 | 恢复路径索引去信任与重建不变量 |
| project-model M06（加行） | D09 要义 | TransactionId AtomicU64 单调分配（数据留 carrier） |
| runbook `.claude/runbooks/test-bench-diagnosis.md`（新建，登记 R27） | K20/K21/K34/K35 | 测试/bench 诊断框架 + baseline 纪律 + tempdir leak + bench 技巧 |
| analysis `.claude/analysis/engine-patterns-legacy-knowledge.md`（新建，登记 R28） | K01-K04/K06-K09/K14-K19/K22-K33 | 26 条现役知识逐字迁移 |
| references R27/R28（新增） | 上两项 | 检索登记 |

## 本次明确排除的条目

- ISS01-03 Artifact-Archive：按判定规则直接移入 `.claude/issues/archive/` 并更新 R21-R23，不经 carrier，不属本批。
- R08-R13 的关联决策/知识字段：历史检索元数据，K/D 编号经墓碑可解析，不回改（归档产物不可变同理）。
- SNAPSHOT 权威文档段 D/K 两行与仓库现场段刷新：SNAPSHOT 归 docs-maintainer 维护，本批不动，已列入移交清单。
- R01/R02 依赖表补全、M01/M05/M10/M11/M12 内容修正：内容维护非生命周期动作，移交 docs-maintainer。

## 侧效应声明

decisions/knowledge 源文件退役后保留 Purpose + Requirement + arc 墓碑空壳（可 validate、历史编号可解析），不删除目录；project-model 同批新增 M17/M18、M06 加行并移除 M07/M16。
