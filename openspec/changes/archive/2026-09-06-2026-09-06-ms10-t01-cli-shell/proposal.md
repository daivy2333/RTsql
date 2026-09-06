# proposal: MS10-T01 CLI 壳——参数化入口与 one-shot 主命令

> **扩展（2026-09-06，用户批准）**：Iteration 000 Review 揭示引擎级真投影缺口（IndexScan 表头错位 + 聚合静默 Null + ORDER BY 失效，同根因三症状），用户决策纳入本 change 作为 Iteration 001（方向 B：执行器真投影；单 Iteration 全做）。详见 What Changes 增补段。

## Why

RTsql 当前唯一二进制是硬编码 demo（`src/main.rs:10-27`：固定打开 CWD `rtsql.db` → `.ok()` 吞错建 test 表 → 监听 `127.0.0.1:9876`）。无参数解析、无环境变量、无 one-shot 执行——非开发者与 agent 无法使用（R18 主题 1）。MS10-T01 是应用层主轨的第一个 change，为后续 T02-T05（文件锁、格式头、多语句、生命周期子命令）和 MS11-MS13 建立载体。

用户 2026-09-06 已批准 roadmap（MS10 优先于 MS08 剩余项）；本 change 前置四项执行决策（同日本会话）：

1. **执行路径**：CLI 组合 `pub` 的 `parse_stage`/`plan_stage`/`execute_stage` 三阶段，不调 `execute_sql`——直接取得列名表头、在执行前检测多语句、打开期错误保留类型。CLI 路径不查 PlanCache（一次性命令无感）。
2. **JOIN 表头**：`get_plan_output_columns` 补 Join/SemiJoin/AntiJoin 三臂（`OutputColumn` 有现成 `column` 字段，每个臂 1-3 行）。
3. **裸名解析**：裸名 `foo` → `$RTSQL_HOME/db/foo.db`，`RTSQL_HOME` 默认 `~/.rtsql/`（R18 口径；根目录将来容纳 `keys/`）。与 tasks.md Outcome 字面口径不一致，以本决策为准，docs 收尾时对齐。
4. **JSON 形状**：`{"columns":[...],"rows":[[...],...]}`——自描述，agent 不靠位置猜列。

## What Changes

- **CLI 壳（新建 `src/cli/` + 重写 `src/main.rs`）**：clap 参数化入口 `rtsql <db> <sql> [--format table|json|csv|tsv]`；名称解析（含 `/` 为路径原样，裸名走集中存储）；输出渲染四格式（TTY 默认 table、非 TTY 默认 json）；退出码分类（0 成功 / 2 用法 / 3 SQL 错 / 4 锁冲突 / 5 密钥，4 与 5 本 change 仅建枚举留位）；正常退出前 `Database::close()`（checkpoint）。
- **列名表头（改 `src/parser/planner/query.rs`）**：`get_plan_output_columns` 补 3 个 JOIN 臂。
- **`value_to_json` 可见性**：`pipeline.rs` 私有 fn → `pub(crate)`，CLI JSON 渲染复用（NaN/Inf→Null 语义沿用）。
- **多语句临时护栏**：`parse_stage` 返回多条时 CLI 显式报错退出 3（文案指向 MS10-T04），替代 `execute_sql` 内部 `first()` 静默截断；T04 落地 `;` 分片逐条执行后此护栏自然退役。
- **lib 面**：`lib.rs` 挂 `pub mod cli`。

### Iteration 001 增补：扫描执行器真投影（用户决策 2026-09-06，方向 B）

- **根因**：四个扫描执行器（Scan/DataScan/IndexScan/IndexScanAll）恒返回全 schema 行；planner 在 PK 点查路径给 `IndexScanNode.columns` 塞投影子集（`query.rs:308`）——元数据与行形状不一致被三类消费方当真：
  1. CLI/表头：`SELECT name ... WHERE id=1` 输出 `columns:["name"]` + 双字段行；
  2. 聚合：`input_schema` 对 IndexScan 输入落入 `_ => vec![]` 兜底（`query.rs:469-471`），`extract_value` 对缺失映射静默返回 `Value::Null`（`aggregate.rs:271-274`）——`SELECT SUM(price) FROM s WHERE id=2` 返回 `[[null]]`（实测复现，正确值 20）；
  3. ORDER BY：`SortExecutor` 按投影列集 `position` 查找排序列，投影外排序键静默退化 `Ordering::Equal` 不排序（实测复现）。
- **修复**：执行器按投影裁剪行（真投影）。planner 把投影列下放到四个 scan 节点；谓词（WHERE/下推/JOIN 条件）在全 schema 行上先行求值，投影发生在行产出最后一步（谓词 `column_index` 按全 schema 解析的既有语义不动）。聚合/排序的列映射随之自然对齐。
- **范围**：四个扫描执行器 + plan 节点构造/pipeline 传参 + 聚合 `input_schema` 与 Sort 列映射对齐 + 既有测试中全 schema 断言的校准（**此 Iteration 允许修改既有测试断言**——行为变化是本 Iteration 的目标语义，与 Iteration 000 的"零修改"不变量按 Iteration 边界区分）。
- **排除**：JOIN 的投影裁剪（`build_output_row` 已按 `output_columns` 提取，天然投影）；派生表/子查询的重投影；`SELECT *`（投影=全 schema，行为不变）。

### Out of Scope（本 change 不做）

- 文件锁 / 信号停机（T02）；magic/格式版本头（T03）；多语句 `;` 分片执行（T04）；生命周期子命令 new/list/schema/dump/restore/import（T05）。
- REPL、鉴权、加密密钥实际逻辑（MS12）、表达式/函数层（MS11）、分发打包（MS13）。
- PG 网络服务路径：`main.rs` 的 server demo 被替换后，server 代码保留为库能力（R18 主题 4 结论），不在本 change 触动。
- PlanCache 接入 CLI 路径（正确性无影响，重复 SQL 重 plan，一次性命令无感）。

## Impact

- **新增**：`src/cli/mod.rs`（或拆 `args.rs`/`resolve.rs`/`render.rs`，Act 定）、`tests/cli_test.rs`（binary 集成测试：`env!("CARGO_BIN_EXE_rtsql")` + `std::process::Command`，零新 dev-dependency）。
- **修改**：`src/main.rs`（整体重写为 CLI 入口）、`src/lib.rs`（+1 行）、`src/parser/planner/query.rs`（3 个 match 臂）、`src/pipeline.rs`（1 处可见性）、`Cargo.toml`（+clap）。
- **行为变化**：binary 从"启动 PG server"变为"one-shot 执行后退出"；查询输出首次带真列名表头。
- **兼容性**：585 既有测试零修改全绿（`main.rs` 无测试引用；`get_plan_output_columns` 现仅被派生表列注册消费，JOIN 臂从"空 Vec"变"真列名"只影响派生表场景的列注册信息，经查 JOIN 不可作为派生表输入，实际零影响）；网络路径与 lib API 零变化。
- **风险**：
  - 打开不存在路径 = 静默创建空库（现状行为，sqlite3 同款）——本 change 保持不提示，风险记录。
  - `Database::open` 失败（IO 错、页对齐错、redo 失败）的退出码 tasks.md 未覆盖——本 change 默认假设：归 1（一般错误），Plan Review / docs 收尾时复核。
  - clap 为新增依赖（MS10 规划已批准 `clap 依赖 + src/cli/`），版本锁定 4.x。
