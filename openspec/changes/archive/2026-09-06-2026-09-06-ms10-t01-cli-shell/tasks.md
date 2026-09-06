# tasks: MS10-T01 CLI 壳

> Iteration 000（CLI 壳）已完成（Review accepted，2026-09-06）；Iteration 001（扫描执行器真投影）已完成（Review accepted，2026-09-06）——为本 change 扩展（用户批准 2026-09-06，方向 B 单 Iteration 全做）。MS10-T02/T03/T04/T05 不在本 change。

## Iteration Plan

### Iteration 000: CLI 壳全链路可用 — completed

- Tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: `rtsql <db> <sql>` 端到端稳定（名称解析、四格式渲染、列名表头、退出码分类、close 语义）；binary 集成测试与渲染/解析单测全绿；585 既有测试零修改
- Verification boundary: `cargo test --all` 全绿（新增 cli_test 集成 + 渲染/解析单测）；`cargo clippy -D warnings` 0；`cargo fmt --check` 0；`openspec validate` PASS
- Diagnostic boundary: `src/cli/`、`src/main.rs`、`src/parser/planner/query.rs`（JOIN 表头臂）、`src/pipeline.rs`（可见性）
- Non-goals: 文件锁/信号（T02）、格式头（T03）、多语句分片（T04）、生命周期子命令（T05）、REPL、PlanCache 接入 CLI、网络 server 路径改动
- Status: **completed**（Review accepted 2026-09-06；608 tests pass，608 = 585 + 11 lib + 12 cli_test）

### Iteration 001: 扫描执行器真投影 — completed

- Status: **completed**（Review accepted 2026-09-06；614 tests pass，614 = 608 + projection_test 6）
- Tasks: T7, T8, T9, T10
- Depends on: Iteration 000（CLI 使投影语义可观察，Review 中的三个实测复现即经 CLI binary）
- Stable baseline: `SELECT <子集投影>` 在全部扫描路径返回投影列的表头与行；PK 点查聚合返回正确值；投影外排序键正确排序；既有测试校准后全绿
- Verification boundary: `cargo test --all` 全绿（含校准后的既有断言）+ 三症状的定向回归测试（表头/聚合/排序各至少 1 个）
- Diagnostic boundary: `src/executor/{scan,data_scan,index_scan,index_scan_all}.rs`、`src/parser/planner/query.rs`（节点构造与列映射）、`src/pipeline.rs`（executor 传参）
- Non-goals: JOIN 投影裁剪（已天然投影）、派生表/子查询重投影、`SELECT *` 语义、列裁剪的性能优化论证、planner 代价模型

**平衡审计**：四个执行器投影 + 列映射对齐 + 测试校准共同形成单一可验收结果（"投影语义正确"），症状三合一、根因单一，拆分会产生"部分投影"中间态（部分路径裁剪部分不裁剪，表头与行形状跨路径不一致）。不拆分。

**规划依据**：Iteration 000 Plan Review Findings 1/2（NEW-EVIDENCE）+ 用户 2026-09-06 探索会话实测复现（聚合 `[[null]]`、ORDER BY 静默失效）；方向 B 决策（真投影优于元数据对齐）。

## Tasks

### T1: JOIN 列名表头臂

- Requirement/Scenario: R3 / S“JOIN 查询表头”、“JOIN 臂补齐不影响既有派生表路径”
- Targets: `src/parser/planner/query.rs::PlanBuilder::get_plan_output_columns`
- 当前行为: Join/SemiJoin/AntiJoin 臂返回 `Vec::new()`
- 目标行为: 返回 `node.output_columns.iter().map(|c| c.column.clone()).collect()`
- 测试见证: 新增单测（`query.rs` `#[cfg(test)]` 或 `tests/cli_test.rs` 前置单测）：构造 JOIN plan 断言列名序列；既有全量回归
- Forbidden: 不改 `OutputColumn` 结构、不动执行器

### T2: 名称解析

- Requirement/Scenario: R2 / 全部 4 个 Scenario
- Targets: 新建 `src/cli/resolve.rs`（`resolve_db_path`）
- 当前行为: 不存在
- 目标行为: 含 `/` 原样；裸名 → `$RTSQL_HOME|~/.rtsql/` + `/db/` + `<name>.db`；HOME 与 RTSQL_HOME 均缺失时报错
- 测试见证: 单测覆盖 4 场景（裸名默认 / RTSQL_HOME 覆盖 / 路径直开 / HOME 缺失报错；env 测试用串行或 `std::env::remove_var` + 恢复）
- Forbidden: 不做 `~` 字面量展开、不创建目录（`Database::open` 自建文件，但父目录不存在时报错即退出 1）

### T3: 渲染四态

- Requirement/Scenario: R4 / 全部 5 个 Scenario + R3 表头场景
- Targets: 新建 `src/cli/render.rs`（纯函数：`render_table/render_json/render_csv/render_tsv` + `OutputKind`）
- 当前行为: 不存在
- 目标行为: 按 design D5 语义渲染（table 对齐、json columns+rows 对象、csv RFC4180、tsv 转义、NULL 空串、Bool true/false、DML affected_rows）
- 测试见证: 单测逐格式断言输出文本（含转义边界：`a"b,c`、内嵌 `\t`/`\n`、NULL、Bool、AffectedRows）
- Forbidden: 渲染函数不做 IO、不查环境、不改 `Response` 类型本身

### T4: CLI 编排与退出码

- Requirement/Scenario: R1 / 全部 4 个 Scenario + R5 多语句拒绝
- Targets: 新建 `src/cli/mod.rs`（`run()` + `ExitStatus`）、改 `src/main.rs`、`src/lib.rs`（+`pub mod cli`）、`src/pipeline.rs`（`value_to_json` → `pub(crate)`）、`Cargo.toml`（+clap 4）
- 当前行为: main 是硬编码 server demo
- 目标行为: 按 design D1/D4/D6/D7——clap 参数、三阶段组合、多语句护栏（len>1 报错退出 3）、列名提取、渲染分发（TTY/非 TTY 默认）、`close()` 正常+错误路径、退出码枚举含 4/5 留位
- 测试见证: `tests/cli_test.rs` 集成测试（真二进制）：成功 SELECT（退出 0 + 表头）、用法错误（2）、SQL 错（3）、多语句拒绝（3 且零执行）、非 TTY 默认 JSON、--format csv 转义、DML affected_rows、close 后 WAL 截断（文件长度收缩或 redo 为空的等价断言）
- Forbidden: 不实现锁/密钥逻辑（仅枚举）、不做信号处理、不接 PlanCache、不动 `execute_sql`/`execute` 编排器
- 依赖: T1-T3（编排消费三者）

### T5: 打开不存在路径与 open 失败路径

- Requirement/Scenario: R2 / “打开不存在路径创建空库”
- Targets: `src/cli/mod.rs`（open 错误 → 退出 1）+ 集成测试用例
- 当前行为: 不存在（main demo 吞错）
- 目标行为: 不存在路径静默建库成功；权限/页对齐/redo 失败 → stderr 错误 + 退出 1
- 测试见证: 集成测试：对新名执行 CREATE TABLE 成功退出 0；对页不对齐文件（写入 100 字节垃圾）报错退出 1
- 依赖: T4

### T6: 全量回归与收尾验证

- Requirement/Scenario: 全部（回归门）
- Targets: 无新代码；跑 `cargo test --all`、`cargo clippy -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t01-cli-shell`
- 测试见证: 命令输出与退出码记入 Act Response
- 依赖: T1-T5
- Status: **completed**（Iteration 000，2026-09-06）

## Iteration 001 Tasks

### T7: plan 节点携带投影 + 执行器按投影裁剪行

- Requirement/Scenario: IR1（真投影语义）/ S1-S4
- Targets: `src/parser/planner/query.rs`（IndexScanNode 构造 `:308` 及 DataScan/Scan 节点 columns 语义）、`src/executor/{scan,data_scan,index_scan,index_scan_all}.rs`、`src/pipeline.rs`（executor 构造传参 `:435-473`）
- 当前行为: 四执行器恒返回全 schema 行；IndexScanNode.columns=投影子集（元数据谎言）
- 目标行为: 四 scan 节点的 `columns` 语义统一为**输出投影**；执行器在全 schema 行求值谓词后按投影裁剪产出；`SELECT *` 时投影=全 schema（行为不变）
- 关键约束: DataScan 下推谓词的 `column_index` 按全 schema 解析（`expression.rs:114`，`self.tables` 注册全 schema）——**投影必须发生在 `filter_row` 求值之后**（`data_scan.rs:336/356` 两个行产出点）；IndexScan 点查行同样先全 schema 反序列化再裁剪
- 测试见证: RED——`tests/projection_test.rs`（新）子集投影断言：`SELECT name FROM s`（DataScan）与 `SELECT name FROM s WHERE id=1`（IndexScan）都返回单列行 `[["Alice"]]`；GREEN 后全量回归
- Forbidden: 不改谓词解析语义、不动 JOIN `build_output_row`、不做列裁剪的性能论证

### T8: 聚合 input_schema 与 Sort 列映射对齐

- Requirement/Scenario: IR1 / S5（聚合正确）、S6（排序正确）
- Targets: `src/parser/planner/query.rs:469-471`（`input_schema` 兜底）、`src/executor/sort.rs`（排序列查找 `:59-63`）、`src/parser/planner/query.rs:522-527`（SortNode.columns 构造）
- 当前行为: 聚合对 IndexScan 输入列映射为空 → `SUM(price)` 静默 Null；Sort 按投影列集 position 查找，投影外排序键静默不排序
- 目标行为: 聚合 `column_indices` 从输入 plan 的**真实输出列**（=投影后）构建且与行形状一致；Sort 的排序列解析基于实际行形状；投影外排序键（`SELECT id FROM s ORDER BY name`）在投影语义下要么正确排序（保留全 schema 求值再裁剪）要么显式报错——**设计取向：正确排序**（排序键不需要出现在投影中，SQL 标准行为）
- 关键约束: T7 后输入行=投影形状，聚合 `column_indices` 必须按投影列构建（`get_plan_output_columns` 即为此用）；排序键若不在投影内需在裁剪前取值（执行器持有全 schema 行时先比较后裁剪，或 Sort 节点携带排序键的全 schema index）——Act 在契约内选实现
- 测试见证: RED——`SELECT SUM(price) FROM s WHERE id = 2` 返回 `20`（现在 `null`）；`SELECT id FROM s WHERE price > 15 ORDER BY name DESC` 按语义排序（现在静默原序）
- Forbidden: 不引入显式报错替代正确排序（那是对 SQL 语义的裁剪）

### T9: 既有测试校准 + 三症状定向回归

- Requirement/Scenario: IR1 / S7（回归与校准）
- Targets: `tests/`（`executor_test.rs`、`pipeline_test.rs`、`plan_exec_test.rs` 等含全 schema 行断言的用例）、`tests/cli_test.rs`（受投影影响的断言）
- 当前行为: 大量既有断言假设"SELECT 子集投影返回全 schema 行"
- 目标行为: 断言校准为投影语义；三症状各有定向回归用例（表头/行一致、PK 点查聚合、投影外 ORDER BY）留在 `tests/projection_test.rs`
- 校准纪律: 逐个核对受影响断言，只改行形状/列数期望，不改测试意图；校准清单（文件→用例→变更）记入 Act Response；`cli_test` 中 Iteration 000 为绕开错位而用全 schema 投影的用例（④/⑥）改回子集投影以锁定新语义
- 测试见证: 校准前后 `cargo test --all` 对比；校准后全绿
- Forbidden: 不删测试、不为通过而弱化断言语义

### T10: 全量回归与 Iteration 验证门

- Requirement/Scenario: 全部（回归门）
- Targets: 无新代码；`cargo test --all`、`cargo clippy -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t01-cli-shell`
- 测试见证: 命令输出与退出码记入 Act Response
- 依赖: T7-T9
- Status: **completed**（Iteration 001 Cycle 000，2026-09-06；614 tests pass / clippy 0 / fmt 0 / validate PASS）
