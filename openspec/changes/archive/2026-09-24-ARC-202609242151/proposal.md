# proposal: ARC-202609242151 初版后清理批——已完成/已排期 improvements 与 tasks 已完成路线归档

## Why

用户 2026-09-24 触发 openspec-archivist 清理并两轮批准：第一轮批准 improvements 24 条（16 条已实施 promoted + 8 条已排期）与 tasks 长期完成项归档、m19/m21 两份已替代分析 Artifact-Archive；第二轮扩大范围——tasks 内**全部**已完成/已废弃 MS 规划段（MS00-MS17）及其流程图、完成记录表全部归档（用户豁免 30 天阈值：「全都归档就行，没必要保留」「别的文档也是同样的要求」）。已排期 Ixx（I042/I059/I061/I062/I063/I068/I070/I072）按用户同日裁定「已排期工作归宿是 tasks，不再登记 I 文档」改判归档（上一批审计 2026-09-24 曾判 Keep，本批依新指令覆盖）。工作树基线：HEAD `d8165fc` + 未提交 tasks MS23-MS27 规划改动（本批归档抽取含该状态）；`openspec list` 无活跃 change（清理前新鲜验证）。

## 映射表

### Archive（进入本 carrier）

#### 源：`openspec/specs/improvements/spec.md`（24 条，全文见 `archive/improvements.md`）

| 原编号 | 动作 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|
| I014 | Archive | promoted 且 RC 部分已实施（MS09-T01，spec `transaction-isolation-levels`）；Serializable 显式非目标，无活方向 | tasks MS09 段（随本批归档）、spec 溯源注 | Serializable 需求重启时插回 |
| I017 | Archive | promoted 且已实施（MS09-T04，spec `correlated-subquery-cache`） | 同上 | 关联缓存需求重启时插回 |
| I032 | Archive | promoted 且已实施（MS09-T01，spec `mvcc-tombstone-visibility` R4） | tasks MS09 段 9 处、spec 溯源注 | 同上（机制回归时） |
| I033 | Archive | promoted 且已实施（MS09-T01 墓碑 slot 化） | tasks MS09 段 13 处、spec 溯源注 | 同上 |
| I034 | Archive | promoted 且已实施（MS15-T02） | tasks MS15 段 5 处、2 份 spec | 同上 |
| I035 | Archive | promoted 且已实施（MS13-T03，spec `no-from-select`） | tasks MS13 段 6 处 | 同上 |
| I036 | Archive | promoted 且已实施（MS15-T01，spec `planner-key-equality-routing`） | tasks MS15 段 6 处 | 同上 |
| I037 | Archive | promoted 且已实施（MS15-Rest/MS16） | tasks MS15/16 段 3 处、spec `update-index-maintenance` | 同上 |
| I039 | Archive | promoted 且已实施（MS15-Rest，spec `table-name-resolution`） | tasks MS15 段 4 处 | 同上 |
| I040 | Archive | promoted 且已实施（MS11-T01） | tasks MS11 段 4 处 | 同上 |
| I041 | Archive | promoted 且已实施（MS17-T02） | tasks MS17 段 8 处 | 同上 |
| I042 | Archive | 已排期 MS19-T03（2026-09-24），权威位置移至 tasks 任务行（用户裁定：已排期不入 I 台账） | tasks MS19-T03 活跃引用 6 处（arc 墓碑可解析） | MS19 取消排期或需求重启时插回 |
| I043 | Archive | promoted 且已实施（MS13-T02） | tasks MS13 段 6 处 | 同 I042 类 |
| I044 | Archive | promoted 且已实施（MS13-T02） | tasks MS13 段 7 处 | 同上 |
| I046 | Archive | promoted 且已实施（MS16-T01） | tasks MS16 段 7 处 | 同上 |
| I047 | Archive | promoted 且已实施（MS16-T02） | tasks MS16 段 8 处 | 同上 |
| I048 | Archive | promoted 且已实施（MS17-T02） | tasks MS17 段 8 处 | 同上 |
| I059 | Archive | 已排期 MS20（一期 T01/二期 T02），权威位置 tasks | tasks MS20 段 2 处 | MS20 取消/重启时插回 |
| I061 | Archive | 已排期 MS19-T01 | tasks MS19 段 2 处 | 同上 |
| I062 | Archive | 已排期 MS18-T01 | tasks MS18/MS26 段 4 处 | 同上 |
| I063 | Archive | 已排期 MS18-T02 | tasks MS18 段 2 处 | 同上 |
| I068 | Archive | 已排期 MS21-T01/T02 | tasks MS21 段 2 处 | 同上 |
| I070 | Archive | 已排期 MS19-T02 | tasks MS19/MS23 段 2 处 | 同上 |
| I072 | Archive | 已排期 MS19-T01 顺带 | tasks MS19 段 3 处 | 同上 |

#### 源：`.claude/docs/tasks.md`（6 块，全文见 `archive/tasks-completed-roadmap.md`）

| 原条目 | 动作 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|
| 『已完成历史』节 + MS00 段 | Archive | 长期完成（2026-05-24，123 天），用户豁免阈值指令「全都归档」 | R08-R13 索引 | 历史追溯时经本 carrier |
| MS01-MS17 全部段（18 段：12 completed + MS03/04/05/08/12/14 共 6 superseded） | Archive | 用户 2026-09-24 指令：已完成 MS 规划无保留价值，全部归档（含 29 天 MS06——阈值豁免）；superseded 段被 I031/I051-I058 等条目按编号引用，经 arc 墓碑可解析 | improvements 保留条目按 MSxx 编号溯源、specs 无路径依赖 | 需要历史规划上下文时经本 carrier |
| 依赖关系图已完成历史流程图（MS00→MS17 + 独立 MS16 行） | Archive | 用户指令点名「相关流程图」随完成规划一并归档 | 无路径依赖 | 同上 |
| 『最近完成』表全部 36 行 | Archive | 全部为已完成工作记录（2026-06-03～09-24），权威已在 SNAPSHOT 仓库现场/git 历史/归档 carrier | 无 | 同上 |
| 规划理念段（2026-09-06～09-14 重排叙述） | Archive | 纯历史重排叙述，无活跃约束 | 无 | 同上 |
| 执行顺序段（既定执行序 ✅ 记录） | Archive | 已完成执行序记录，随完成规划一并归档 | 无 | 同上 |

### Artifact-Archive（不进 carrier，`.claude/analysis/archive/`）

| 文件 | 动作 | 判断理由 | 交叉引用 | 恢复条件 |
|---|---|---|---|---|
| `m19-datascan-path.md` | Artifact-Archive | R06 判 completed，内容沉淀 R28/M02；已替代不再使用（未达 180 天阈值，用户批准按「同样要求」提前） | R06（本批补归档路径注） | R06 路径注定位 |
| `m21-page-visibility-incomplete.md` | Artifact-Archive | R07 判 completed，现行权威 spec `mvcc-tombstone-visibility` + M10/M17 | R07（同上） | 同上 |

## 本次明确排除的条目

- improvements 保留 33 条（I012/I015〔SMJ 残留〕/I016/I020/I021/I024/I025/I026/I028/I029/I030/I031/I038/I045/I049/I050/I051-I058/I060/I064/I065/I066/I067/I069/I071/I073/I074）：均为未排期活方向或用户裁定保留项。
- tasks 活跃路线：MS18-MS27 十段、统一执行序、D-candidates、长期方向、进行中/已承诺待办/阻塞结构节。
- references 全部 R01-R28：检索元数据本身即权威索引；R06/R07 仅补归档路径注，不归档。
- analysis 活跃 5 份（engine-patterns-legacy-knowledge〔R28〕、ms10-t03〔R19〕、ms10-t05〔R20〕、usability-gap-cli-form〔R18〕、workspace-crate-modularization〔R25〕）与 runbooks 4 份（R17/R24/R26/R27）：时效内且活跃引用。
- SNAPSHOT/CLAUDE.md：不动（SNAPSHOT 刷新属 Maintainer；CLAUDE.md 永不自动归档）。
- project-model：当前约束全 Keep。

## 侧效应声明

carrier 归档成功后：`openspec/specs/improvements/spec.md` 精准移除 24 条目并追加 `<!-- arc: ARC-202609242151 -->` 墓碑；`.claude/docs/tasks.md` 移除 6 块并追加同 ID 墓碑（路线图结构计数段、统一执行序尾部、依赖关系图收尾句、「最近完成」节注释同步改写为归档后口径）；R06/R07 补 `[ARCHIVED 2026-09-24]` 路径注；两份分析文件移入 `.claude/analysis/archive/`。
