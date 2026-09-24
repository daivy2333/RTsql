# mvcc-tombstone-visibility Specification

## Purpose

约束 MVCC 版本链中墓碑（删除标记）的可见性评估语义：墓碑 SHALL 自描述其删除者，已提交墓碑 SHALL 抑制整条版本链，未提交/已回滚墓碑 SHALL NOT 抑制、评估回溯前驱；运行期与崩溃恢复后两态一致。来源：MS09-T01（I033，MS15-Rest 收尾跨进程探针实证 + Plan Review 独立复现 2026-09-12；change `2026-09-13-ms09-engine-mvcc-closeout`）。调查基线事实：现状 DELETE 就地改写最新版本 header（哨兵覆写 `commit_tx_id`，删除者 tx_id 不保留），已提交与未提交墓碑在页面上不可分辨——修复以「墓碑独立版本 slot（自带删除者）」替代就地标记。

## Requirements

### Requirement: 墓碑自描述且不影响前驱版本

DELETE SHALL 以独立墓碑版本表达（删除者事务 id 保留于墓碑自身 header，链指针指回被删版本），SHALL NOT 就地改写被删版本的 header；被删版本的 header（含提交信息与链指针）SHALL 保持删除前状态。

#### Scenario: 未提交删除期间扫描见删除前已提交版本

- **GIVEN** 行最新已提交版本为 (1,99)；另一连接在未提交事务内 DELETE 该行
- **WHEN** 并发连接（默认路径、无快照扫描）扫描
- **THEN** 产出 (1,99)（删除前最新已提交版本；修复前产出 (1,10)——回溯越过 pre-delete 版本，调查实证的同根未提交形态）

#### Scenario: 被删版本 header 不被改写

- **GIVEN** 同上，未提交 DELETE 执行后
- **WHEN** 读取被删版本的 VersionHeader
- **THEN** 其提交信息与链指针与删除前一致（就地哨兵覆写消除）

### Requirement: 已提交墓碑抑制整条版本链

墓碑的删除者事务对当前读取已提交（无快照读取时删除者不在活跃集合，或 RC 视图下语句开始前已提交）时，该行 SHALL 整体不可见——SHALL NOT 沿链回溯产出该行任何前驱版本。扫描路径（数据页链扫描）与既有索引路径 SHALL 语义一致。

#### Scenario: update→delete 两步变更扫描不重现旧版本（I033 探针序列）

- **GIVEN** 进程 1 `INSERT (1, 10)` 提交；进程 2 `UPDATE SET n=99 WHERE id=1` 提交（affected 1）；进程 3 `DELETE WHERE id=1` 提交（affected 1）
- **WHEN** 新进程（无快照扫描）`SELECT * FROM t` 与点查 `WHERE id=1`
- **THEN** 扫描空集且点查空集（修复前扫描 `[[1,10]]` 而点查空集——两步变更从扫描面消失）

#### Scenario: 异键 update→delete 对照保持

- **GIVEN** 行 (1,10)；`UPDATE SET id=2 WHERE id=1` 提交；`DELETE WHERE id=2` 提交
- **WHEN** 扫描
- **THEN** 空集（既有正确形态 Z1 保持）

#### Scenario: delete→update 既有形态保持

- **GIVEN** 既有正确形态 Z2 的建链序列
- **WHEN** 扫描
- **THEN** 结果与修复前一致（零回归锚点）

### Requirement: 未提交与已回滚墓碑不抑制、评估回溯前驱

删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。

#### Scenario: 墓碑事务回滚后扫描恢复

- **GIVEN** 行最新已提交版本 (1,99)；另一连接 DELETE 该行后 ROLLBACK
- **WHEN** 扫描
- **THEN** 产出 (1,99)（回滚中性化；回滚前未提交期间同为此形）

#### Scenario: 自身事务内删除对自己可见为已删

- **GIVEN** RC 模式显式事务内 DELETE 行后
- **WHEN** 同事务内 SELECT
- **THEN** 该行不可见（自身删除按已删语义）

### Requirement: 恢复后墓碑语义一致

崩溃恢复（WAL 重放）后的墓碑评估 SHALL 与运行期一致：已提交删除序列重启后扫描 SHALL 空集；崩溃时未提交的删除（其 WAL 记录不被重放）SHALL 不生效。恢复期对未提交事务遗留版本 SHALL 显式中性化标记（I032 实施）：未提交行在重启后对无快照扫描 SHALL NOT 复活。

#### Scenario: restart 后 I033 序列扫描空集

- **GIVEN** I033 探针序列完成后关闭并重新打开数据库
- **WHEN** 扫描与点查
- **THEN** 空集（与重启前一致）

#### Scenario: 崩溃时未提交 DELETE 恢复后不生效

- **GIVEN** 行 (1,99) 已提交；未提交 DELETE 后进程终止
- **WHEN** 重启后扫描
- **THEN** (1,99) 可见（未提交删除的 WAL 记录不被重放）

#### Scenario: 落盘未提交行重启后不复活（I032）

- **GIVEN** 未提交事务的 INSERT 行已页级落盘（驱逐或 checkpoint），进程终止
- **WHEN** 重启后无快照扫描
- **THEN** 该行不可见（恢复期未提交标记实施；修复前复活）

### Requirement: 页级可见性摘要无毒化（哨兵语义）

页级可见性摘要（`PageVisibilityInfo`）SHALL 以 `min_create_tx_id = MIN_CREATE_UNKNOWN`（`u64::MAX`）表达「最小创建事务 id 未知」：`all_invisible_for` SHALL 对 UNKNOWN 返回 false（信息不足 → 回落逐行检查），SHALL NOT 因哨兵值判定整页不可见。写路径（INSERT/DELETE/UPDATE/COMMIT/恢复重放）对无条目页首次建立摘要条目时 SHALL 携带 UNKNOWN 哨兵而非 0（`clear_all_visible` 的 `or_insert` 形态）；INSERT 路径随后合并真实 `create_tx_id` 后 SHALL 反映真实最小值。已置 `all_visible` 的页经写路径清除后 SHALL 回落到逐行检查（`all_invisible_for` 为 false），SHALL NOT 出现「哨兵残留 + all_visible=false」导致整页误判不可见的形态。本 Requirement 的消费面为快照携带执行（Read Committed 隔离级别）；Repeatable Read 默认路径无快照语义不受影响。

#### Scenario: INSERT 首建条目反映真实最小事务 id（ISS01 修复）

- **GIVEN** 无可见性摘要条目的数据页（新建页或重启后 vis_map 为空）
- **WHEN** 事务 W 对该页执行 INSERT（`clear_all_visible` → `update_visibility_on_insert(W)`）
- **THEN** 该页条目 `min_create_tx_id == W`（非 0），`all_invisible_for(t)` 对 `t < W` 为 true、对 `t >= W` 为 false

#### Scenario: all_visible 置位后写路径清除不整页误判（MAX 毒化修复）

- **GIVEN** Read Committed 模式下某已提交页经全页扫描惰性置位 `all_visible=true`（条目首建携带 UNKNOWN 哨兵）
- **WHEN** 对该页执行 DELETE 或 UPDATE（`clear_all_visible` 仅清除 `all_visible`）
- **THEN** 该页条目 `all_invisible_for(hw)` 为 false（哨兵识别为未知），页内其余已提交行对后续 Read Committed 语句的点查与全扫保持可达

#### Scenario: RC 端到端 scan→delete→点查可达

- **GIVEN** `open_with_isolation(ReadCommitted)` 打开的库，表 `t(id INT PRIMARY KEY, n INT)` 含三行已提交数据
- **WHEN** 同一连接先执行全表扫描（触发惰性置位），随后 DELETE 其中一行，再对另一未删行执行键位点查与全表扫描
- **THEN** 点查返回该行、全扫返回其余两行（SHALL NOT 因页级摘要毒化丢失行）

#### Scenario: 既有可见性语义零回归

- **GIVEN** 既有 `tests/mvcc_tombstone_visibility_test.rs`、`tests/isolation_level_test.rs` 与 `page_visibility.rs` 既有单测
- **WHEN** 全量测试运行
- **THEN** 全部零修改通过（`PageVisibilityInfo::Default` 语义保持 `{min_create_tx_id: 0, all_visible: false}`，`all_invisible_for` 对真实值 > 快照为 true 的既有断言不变）

### Requirement: 既有 MVCC 语义零回归

非墓碑形态的可见性（已提交非墓碑替代者抑制、不可见沿链查找、RR 默认路径现状语义）SHALL 保持；修复 SHALL NOT 通过禁用页级摘要或全局退化方式实现。既有全量测试 SHALL 零修改通过（依赖旧缺陷行为的既有测试如有，按校准记录处理）。

#### Scenario: 全量回归零修改

- **WHEN** 默认配置运行全量测试
- **THEN** 既有基线零修改通过

**已知边界（记录，不属本 change 验收面）**：未提交删除期间与 DELETE 回滚后的 **PK 点查**不可达为预存索引时序边界（索引条目删除即时、回滚不恢复条目，修复前同此形态）；其修复涉及延迟索引移除与同事务 delete+insert-same-key 语义，作为 Issue 候选另行处置。
