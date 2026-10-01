# catalog-durability-and-cli-surface — Proposal

## Why

2026-09-30 用户组织的功能演示与 0.2.0 实测（本会话 Explorer 调查，全部证据经实际运行与页级取证）发现四组问题，其中一组为最高优先级数据正确性缺陷：

- **P0 顺序写入导致静默丢数据 + 页链损坏（正确性红线）**。跨进程背靠背写入（one-shot CLI 主用法：每轮一个进程 INSERT ~1000 行）从约第 11-12 轮、累积过万行起，已提交数据对扫描/dump 永久不可见，而点查与 MIN/MAX 正常；count 冻结、文件虚胖（4.1MB/1.2 万行）、全程 exit 0 零报错。根因已定位到字节级：
  1. `src/storage/page_format/slotted_page.rs` `delete_slot` 只压缩 slot 数组，`free_space_offset` 永不回退——每次「追加新行 + 删旧行」式更新永久泄漏行宽字节；
  2. `src/storage/catalog.rs:736-813` `update_field_in_chain` 在页满（`add_slot` 失败）时**静默 `return Ok(())`**（注释自认 "extremely unlikely"）；
  3. `update_table_tail` 每次数据页自动分配触发一次（`src/storage/data/table_manager.rs:1503-1508`），~1000 行/轮 ≈ 12 次更新/轮，`__tables` 页约 95-100 次更新后窒息（损坏库尸检：活行 logical_id=112、数据区仅剩 4 字节）；
  4. 此后每个新进程读到陈旧 tail → 各自分配孤儿页段并覆写旧尾页 next（实测 page 332.next 直指第 30 轮首新页 984，最后一次写入者幸存）——扫描链（head→next）只见头部段与最后一轮段，索引树根早期已定型故点查完好。
  5. 同一静默路径还服务于 `update_table_root` / `update_unique_index_root` / `update_secondary_index_root`（B-tree 根分裂写回，`src/storage/btree/index_manager.rs:126-135`）与 `append_secondary_index`（CREATE INDEX 注册，行增长）——窒息后任何根分裂与索引注册都不再持久化，属同族正确性地雷。
- **P1 表别名限定引用家族失效**：表别名在 FROM 构建时被丢弃（query.rs「既有忽略表别名的行为」），所有限定名解析只认真实表名——JOIN 投影别名限定列 `Column not found`、ON/WHERE/GROUP BY 别名被当表名（`Table 't' does not exist`，NLJ 臂又呈 `Table not found: t`）、单表投影别名靠「静默丢弃限定符」意外可用而单表 WHERE 别名仍失败。自连接（必须用别名）场景限定引用完全不可用。
- **P2 dump 对损坏库静默输出残缺备份**：dump 沿数据页链扫描，链断即截断，无任何告警（实测 rc=0 输出 11015 行）——是 P0 的放大器。
- **P3 主命令对不存在的库静默建库**：裸名与路径模式皆建（实测 `rtsql typo_db "SELECT 1"` exit 0 且建文件）；拼写错误静默分叉出新库，文档未提及。生命周期子命令已有 `resolve_existing_db` 拒绝缺失，ATTACH 已有缺失拒绝先例——仅主命令缺口。
- **SKILL.md 命令面不清**：`rtsql-docs/SKILL.md`（377 行）未以命令行形式系统列出当前可用功能；且 P1/P2/P3 落地后命令语义变化需要同步。用户点名要求优化。

四项中 P0 为 one-shot CLI 主用法上的静默数据丢失，属正确性红线，按公共规则「正确性优先」纪律居首。

## What Changes

单 change 五任务面、三 Iteration 承载（MS21/MS26 单 change 多 Iteration 先例）：

- **Iteration 000 — P0 catalog 持久化正确性**（新 spec 域 `catalog-update-persistence`）：
  1. `SlottedPage` 新增**等长原地更新**原语（定位 slot、等长校验、覆写数据区、不动 header）；
  2. `update_field_in_chain` 重构——等长更新（现有全部调用方：tail/root/ordinal root/column_count 均为定宽字段值变化）走原地写回；变长更新（CREATE INDEX 增段等）走链上跨页追加（复用 `append_to_chain` 原语）+ 旧副本清理；**删除全部静默成功路径，任何失败 `Err` 点名传播**；
  3. 跨进程 durability 回归 e2e（多轮 open→写→close 循环 count 单调、dump 完整）与索引根分裂持久化专项（UNIQUE/二级索引分裂后重开仍强制/仍路由）。
- **Iteration 001 — P1 表别名解析**（新 spec 域 `sql-table-alias-resolution` + `order-by-alias-sorting` MODIFIED + `join-output-shape-resolution` MODIFIED）：FROM 别名注册（独立 alias→表映射，四个限定名消费点统一归一化）；限定列解析认别名（SELECT/ON/WHERE/GROUP BY/聚合参数全子句面）；单表限定列与 JOIN 统一（未知限定符点名拒绝，终止静默丢弃）；ORDER BY 限定键支持；三段限定列仅 attach 复合键可达（本地点名拒绝）。
- **Iteration 002 — P2 + P3 + SKILL.md**（`cli-noninteractive-shell` MODIFIED）：dump 前数据链完整性预检（断链 stderr 告警 + rc 0 保持）；主命令打开不存在库显式拒绝（exit 1 + 指引 `rtsql new`）；`rtsql-docs/SKILL.md` 以命令行形式重构功能目录并与 README 双语核验一致。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **P3 目标语义 = 显式拒绝**：主命令（裸名与路径）打开不存在的库 → `ExitStatus::General`（exit 1），错误文案含指引（`database 'x' does not exist; create it first with: rtsql new x`）。创建通道收敛为 `rtsql new`（restore 已要求既有空库，ATTACH 已拒绝缺失）。备选方案（stderr 告警但保持隐式建库 / `--create` 逃生旗标）不采用；lib 层 `Database::open` 建库行为不变（公开 API 兼容）。存量 cli_test 用例随此机械适配（先 `new` 再操作，断言语义不变）。
2. **P2 告警形态 = stderr 告警 + 退出码 0**：dump 仍产出部分备份（备份可用性优先），告警点名「数据页链在 catalog 尾页之前终止，备份可能不完整，库可能已损坏」。备选（exit 1）不采用；完整损坏检测命令留 MS27-T02（`rtsql check`）。
3. **既有损坏库不提供修复工具**：P0 修复后，残尸页上的等长原地更新即刻恢复工作（活行仍在原 slot），尾指针恢复持久化；但已孤儿化的历史数据段不做自动重链/回收。检测能力由 P2 告警与 MS27-T02 承接，修复工具不做（记录为候选方向）。
4. **变长更新的崩溃窗口**：追加新副本与删除旧副本非原子，进程在两步间崩溃会留下同名双副本；以 `scan_tables` 同名去重（取 logical_id 最大 = 最新）闭合歧义，窗口本身文档化接受（与既有单写模型风险同级）。
5. **别名与 ATTACH 优先级**：限定符归一化先查 FROM 表别名映射，再查 `self.tables` 直接键（含 attach 复合键 `alias.table`）；两段式在表引用语境维持既有 attach-first 语义不变。FROM 别名与 attach 别名重名时表别名仅作用于列限定语境，attach 语义不受影响。
6. **单表收紧属目标行为**：`SELECT x.name FROM people`（x 未声明）由「静默丢弃限定符成功」改为点名拒绝；已声明别名的既有用法（`SELECT p.name FROM people p`）修复后经别名映射正常解析，不受影响。既有测试如有依赖静默丢弃形态的按白名单校准。
7. **P1 范围**：覆盖用户报告的三症状（JOIN 投影/ON-当表名/ORDER BY 限定列）及同根的 WHERE/GROUP BY/聚合参数限定面与三段本地拒绝；不含窗口函数、LATERAL 等新能力。
8. **迭代序 = 000 → 001 → 002**：P0 正确性红线优先；001 与 000 无代码耦合（planner vs storage），顺序体现优先级；002 依赖前两者定稿的命令语义（文档与测试校准在后）。
9. **「GROUP BY 不支持表达式」不列入**：实测 `GROUP BY id+0` 可用（MS13 交付），用户报告该项不成立；如另有具体语句形态，另行登记。

## 范围外（Non-goals）

- 既有损坏库的修复/重链工具与数据抢救（假设 3；MS27-T02 `rtsql check` 候选承接检测）。
- slotted page 全量紧凑化/VACUUM（等长原地更新已消除增量泄漏；存量 debris 4KB 级，不值得动页格式）。
- catalog 页分裂/多页 `__tables` 链的容量扩展设计（变长重定位已跨页；页格式不变）。
- SQL 能力新面：窗口函数、LATERAL、标量子查询投影（MS17 边界维持）、递归 CTE。
- `Database::open` lib 级隐式建库语义（公开 API 兼容，仅 CLI 主命令面收紧）。
- dump 告警之外的通用完整性校验命令（MS27-T02）。

## Impact

- **受影响代码面**（Phase 2 调查已复核）：`src/storage/page_format/slotted_page.rs`（原地更新原语）、`src/storage/catalog.rs`（`update_field_in_chain` 重构 + `scan_tables` 同名去重）、`src/parser/planner/query.rs` / `expression.rs` / `src/parser/ast.rs`（别名注册与限定解析四消费点、ORDER BY 键）、`src/cli/lifecycle.rs`（dump 预检）、`src/cli/mod.rs`（主命令存在性前置）、`rtsql-docs/SKILL.md` + `README.md` + `README.zh-CN.md`（文档面）。
- **受影响 specs**：新增 `catalog-update-persistence`、`sql-table-alias-resolution` 两域；MODIFIED `cli-noninteractive-shell`（+2 Requirement）、`order-by-alias-sorting`（R3 边界注记移除 + 限定名排序）、`join-output-shape-resolution`（限定解析含别名）。语料域数 59 → 61。
- **测试**：新增 2 个测试文件（`tests/catalog_durability_test.rs`、`tests/table_alias_resolution_test.rs`）+ 既有文件扩展（order_by_alias_sort / join_output_resolution / cli_test 校准）；全量零回归（change 创建时基线 1590 tests / 0 failures，SNAPSHOT 2026-09-30，M19 分批纪律）。

## 验证边界

- **P0**：catalog 单测（更新循环 ×200 无泄漏、原地写回字节级断言）+ 多进程 durability e2e（lib 15 轮 open→写→close count 单调 + dump 行数一致 + CLI 抽样 12 轮）+ 根分裂专项（UNIQUE 强制与二级索引路由分裂后重开保持）+ 全量零回归。修复前以同测试见证 RED（count 冻结可复现）。
- **P1**：别名矩阵（自连接两别名 × SELECT/ON/WHERE/GROUP BY/聚合参数/ORDER BY × 单表/JOIN/CTE/视图基座）+ 拒绝面（未知限定符/三段本地/attach 优先级）+ 既有读面回归（order_by_alias_sort/join_output_resolution/aggregate_output_shape/derived_scan_alias/cte/no_from_select）。
- **P2/P3**：断链库 dump 告警（页级字节修补夹具）+ 干净库零告警；缺库拒绝矩阵（裸名/路径/`new` 通道/生命周期子命令既有行为/lib API 不变）。
- **文档**：SKILL.md 每条命令行与实现一致（人工核对 + 抽样自动执行），README 双语无冲突。
