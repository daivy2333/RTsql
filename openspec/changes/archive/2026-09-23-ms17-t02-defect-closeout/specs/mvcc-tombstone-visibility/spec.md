# mvcc-tombstone-visibility Specification（delta）

## ADDED Requirements

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
