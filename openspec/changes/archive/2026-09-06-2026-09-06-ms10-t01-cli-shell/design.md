# design: MS10-T01 CLI 壳

## Current Behavior（基线 `709c85d`，工作树代码一致）

- `src/main.rs:10-27` 硬编码 demo：打开 CWD `rtsql.db` → `.ok()` 吞错建 `test` 表 → 监听 `127.0.0.1:9876` → `Server::run()`。无参数、无 env、无 one-shot。
- `Response::QueryResult { rows: Vec<Vec<serde_json::Value>> }`（`src/network/protocol.rs:35`）不带列名；PG 协议伪造 `col0/col1` 表头（`src/network/pg_protocol.rs:254-255`）。
- `execute_sql` 内部 `statements.first()` 静默截断多语句（`src/pipeline.rs:337`）。
- `get_plan_output_columns`（`src/parser/planner/query.rs:23`，`pub(crate)`）：JOIN/SemiJoin/AntiJoin 返回空 Vec（`query.rs:35-38`）。
- `value_to_json`（`src/pipeline.rs:684`）为私有 fn。

## Target Behavior

`rtsql <db> <sql> [--format table|json|csv|tsv]`：解析参数 → 解析名称 → 打开库 → 三阶段执行（带列名提取与多语句检测）→ 渲染 → `close()` → 分类退出码。

## 关键设计决策

### D1：CLI 组合三阶段而非调 execute_sql（用户批准 2026-09-06）

```
parse_stage(sql)                    -> Vec<Statement>     // 多语句检测点（len>1 报错）
plan_stage(db, sql, stmt, false)    -> PhysicalPlan       // plan.clone() 供列名提取
get_plan_output_columns(&plan)      -> Vec<String>        // 表头
execute_stage(db, plan, false)      -> Response           // 执行
```

三个阶段均为 `pub`（`src/pipeline.rs:42/56/96`）。CLI 作为 lib 内模块（`src/cli/`）直接调用，**不查 PlanCache**（正确性无影响；`plan_stage` 内 `put` 照旧发生，只是 CLI 不享受 get 加速——一次性进程无感）。`is_profiling_enabled` 默认关闭，CLI 固定传 `profiling: false`。

### D2：列名表头从 plan 提取，JOIN 臂补齐

表头 = `get_plan_output_columns(&plan)`（`src/parser/planner/query.rs:23`）。现有臂已覆盖 Scan/DataScan/IndexScan(IndexAll)/Aggregate/Having/Filter/Sort/Limit/SubqueryEval/DerivedScan。本 change 补三个臂：

```rust
PhysicalPlan::Join(node) | PhysicalPlan::SemiJoin(node) | PhysicalPlan::AntiJoin(node) => {
    node.output_columns.iter().map(|c| c.column.clone()).collect()
}
```

依据：三个 plan 节点均持有 `output_columns: Vec<OutputColumn>`（`src/executor/plan.rs:304/359/375`），且执行器行组装严格按 `output_columns` 顺序逐项提取（`src/executor/join.rs:112-122` `build_output_row`），列名与行值一一对应。JOIN 现返回空 Vec 的唯一消费方是派生表列注册（`query.rs:81`），而 JOIN 不能作为派生表输入（子查询 body 限 `SetExpr::Select`，无 FROM-JOIN 子查询路径），补臂对既有行为零影响。

表头文本语义（沿用引擎现状，不新造）：普通列 = 列名小写；别名 = 别名（`ExprWithAlias` 优先，`query.rs:409/414`）；无别名聚合 = `result_column_name()` 文本（`COUNT(*)`→`count_star`、`AVG(price)`→`avg_price`，`src/executor/aggregate.rs:48`）。

### D3：模块与入口结构

```
src/cli/mod.rs      // pub fn run() -> ExitCode：参数解析（clap derive）、分发
src/cli/resolve.rs  // 名称解析：resolve_db_path(&str) -> PathBuf
src/cli/render.rs   // 四格式渲染：纯函数（columns + rows + Response → String）
src/main.rs         // #[tokio::main] async fn main() -> ExitCode { cli::run().await }
```

拆分理由：resolve 与 render 是纯逻辑（可单测），run 是编排（集成测试经真二进制覆盖）。Act 可在契约内微调文件边界（如并入 mod.rs），但 render 必须保持纯函数（无 IO），集成测试与单测都以函数边界覆盖。

### D4：退出码映射

| 码 | 含义 | 来源 |
|---|---|---|
| 0 | 成功 | 主命令正常完成 |
| 1 | 一般错误（默认） | `Database::open` 失败、`close()` 失败、渲染期 IO 错误 |
| 2 | 用法错误 | clap 解析失败（clap 默认 exit 2，行为一致）、未知 format |
| 3 | SQL 错误 | parse/plan/execute 阶段的 `Response::Error`、多语句护栏 |
| 4 | 锁冲突 | T02 落地（枚举留位） |
| 5 | 密钥错误 | MS12 落地（枚举留位） |

映射机制：`enum ExitStatus { Success, General(String), Usage(String), Sql(String), Locked(String), InvalidKey(String) }` + `impl From<&ExitStatus> for ExitCode`。clap 自身错误路径已退出 2（`clap::Error::exit`），与枚举一致；集成测试直接断言进程退出码。

### D5：渲染四态

- **输入**：`columns: Vec<String>`、`rows: Vec<Vec<serde_json::Value>>` 或 `AffectedRows(count)`。
- **table**：表头行 + 分隔线 + 数据行，列宽 = 该列最大显示宽度（字节长度近似即可），` | ` 分隔。NULL 渲染为空串。
- **json**：`{"columns":[...],"rows":[[...],...]}`；DML 输出 `{"affected_rows":N}`。数值/字符串/布尔/Null 由 `value_to_json`（改 `pub(crate)`）语义决定；NaN/Inf→Null 沿用。
- **csv**：RFC 4180——含 `,`/`"`/`\n` 的字段引号包裹、内部引号翻倍；NULL 为空字段。
- **tsv**：`\t` 分隔；字段内 `\t`→`\\t`、`\n`→`\\n`、`\r`→`\\r`、`\\`→`\\\\` 转义；NULL 为空字段。
- **Bool 文本**：table/csv/tsv 渲染为 `true`/`false`（JSON 保持布尔）。pg 协议的 `t/f` 是 PG 线上格式，CLI 不沿用。
- **默认值**：`std::io::IsTerminal`（stdout TTY → table；否则 json）。

### D6：名称解析

```
resolve_db_path(arg):
  arg 含 '/'            -> PathBuf::from(arg)
  否则                   -> env RTSQL_HOME（默认 $HOME/.rtsql/）/ "db" / format!("{arg}.db")
```

`~` 字面量不展开（依赖 env 语义）；`$HOME` 未设置且 `RTSQL_HOME` 未设置时报一般错误（退出码 1）。注意 WAL 路径 = 主文件 `with_extension("wal")`（`src/wal/writer.rs:27`）：`foo.db`→`foo.wal`，`foo`（无扩展名路径）→`foo.wal`。

### D7：多语句护栏与 close 语义

- `parse_stage` 返回 `len() > 1` → stderr 报错（文案含"one statement at a time"与 T04 提示）、退出 3、**不执行任何语句、不开库后立即关闭**。
- 主命令正常路径末尾 `db.close().await`：失败报一般错误（退出 1，数据已由 WAL 兜底）；成功退出 0。
- 错误路径（SQL 错误）也执行 `close()`（库已打开时）——保持 WAL 有界，再退出 3。

## 错误与并发语义

- CLI 为单进程 one-shot，无并发面；`Database::open` 内部多任务结构不变。
- 跨进程文件锁是 T02 范围：本 change 两个进程并发开同一文件的行为与现状一致（不检测）。

## 兼容性

- lib 公开 API：仅新增 `pub mod cli`；`value_to_json` 可见性变化是 crate 内部。
- 网络路径零变化（server 代码保留，main 不再引用）。
- 585 既有测试零修改全绿是 Iteration 000 的硬约束。

## Iteration 001 增补设计：扫描执行器真投影（用户决策 2026-09-06，方向 B）

### D8：投影在谓词之后、行产出之时

四个扫描执行器（Scan/DataScan/IndexScan/IndexScanAll）在全 schema 行上完成谓词求值（WHERE/下推谓词）与 MVCC 可见性判定，之后按投影 index 裁剪产出。依据：谓词 `column_index` 由 planner 按全 schema 解析（`expression.rs:114`，`self.tables` 注册全 schema）——裁剪提前会破坏所有谓词。

### D9：节点元数据语义统一为"输出投影"

`ScanNode/DataScanNode/IndexScanNode/IndexScanAllNode` 的 `columns`（或新增投影字段）统一表示输出投影：planner 从 `select.projection` 解析并按全 schema 列序求 index；`SELECT *`/全列投影 = 全 schema（行为不变）。`get_plan_output_columns`/聚合 `input_schema`/`get_subquery_first_column` 等消费方随之读到与行形状一致的元数据。

### D10：排序键与投影解耦

排序键允许不在投影内（SQL 标准语义）。实现取向（非实质，Act 定）：SortExecutor 排序缓冲持有裁剪前行、产出时裁剪；或 SortNode 携带排序键的全 schema index。投影外键 `ORDER BY name` 在 `SELECT id` 下正确排序。

### 影响边界（Iteration 001）

- 改动面：四执行器 + planner 节点构造/列映射 + pipeline 传参 + 既有断言校准。
- 不动面：JOIN（`build_output_row` 已投影）、DML/DDL、子查询重投影、`SELECT *` 行为、MVCC 语义、CLI 编排（Iteration 000 成果零回退）。
- 既有测试校准是目标语义的一部分（区别于 Iteration 000 的零修改不变量），校准纪律：只改行形状期望、不改测试意图、清单完整记录。
