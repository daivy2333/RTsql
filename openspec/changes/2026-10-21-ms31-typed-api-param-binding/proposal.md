# Proposal: MS31 类型化库结果与参数绑定 API（T01 类型化结果与核心/协议解耦 + T02 参数绑定与预编译语句）

- Date: 2026-10-21
- Status: approved（Gate 1 2026-10-21「批准」；Gate 2 计划 2026-10-21「批准」——含 sql-parameter-binding「DML 定位键位占位符点名拒绝」场景修订确认（Phase 2 调查定稿，design D5/R4）与三轮 Iteration Plan；默认假设 A1-A7 生效）
- 来源：milestone MS31（`.agents/docs/tasks.md`，2026-10-19 用户批准底层设计批增补；I088/I089 promoted，2026-10-19 用户裁定优先级排序第 2-3 位）+ 本会话用户裁定（2026-10-21：单 change 承载 T01+T02；`execute_sql`/`execute_in_tx` 旧签名替换 + 全调用面迁移）

## Why

核心执行结果依赖网络词表：`Database::execute_sql`/`execute_in_tx` 直接返回 `network::protocol::Response`，QueryResult 是 `Vec<Vec<serde_json::Value>>` JSON 值数组且不携带列元数据——列名经计划输出列旁路传递给 CLI 渲染、列类型完全不可得；库使用者不能直接消费内部 `Value` 类型，类型与 schema 信息需由调用者另行处理（I088，证据 `src/database.rs::execute_sql`、`src/network/protocol.rs::Response`、`src/pipeline.rs::execute_executor` 与 `value_to_json`）。

程序调用侧：常用库入口只接收 SQL 字符串，调用者需自行渲染值（转义与类型保真成本），重复 SQL 的参数类型与计划生命周期缺少统一公共契约（I089，I083 逐行 SQL 构造为消费例）。

调用面规模（2026-10-21 核对）：src 内 `execute_sql`/`execute_in_tx` 引用约 32 处（`cli/mod.rs`、`cli/lifecycle.rs`、`pipeline.rs`、`transaction/session.rs`、`network/handler.rs`、`storage/integrity.rs`），tests 有 81 个文件消费。SQL 文本当前无占位符语法——`ParameterExpression` 是关联子查询注入的执行器内部机制，非用户可写（2026-10-21 核对：src 无 `Placeholder` 解析路径）。

## What Changes

1. **T01 类型化公共结果与核心/协议解耦**：库执行入口返回类型化结果——查询返回原生 `Value` 行集合与逐列元数据（列名 + 类型），DML 返回受影响行数，错误经 `Result::Err` 携带消息与 SQLSTATE 分类（与既有错误面同源）；`network::protocol::Response` 退化为 CLI/Server 边界转换词表；`execute_sql`/`execute_in_tx` 旧签名替换（用户裁定 2026-10-21），src 调用面与 tests 81 文件迁移（测试断言可经边界转换 helper 保持既有形态）；CLI table/json/csv/tsv 与 PG/JSON 协议输出逐字节零变化。
2. **T02 类型化参数绑定与可复用预编译语句**：SQL 文本引入位置式占位符 `?`（默认假设 A6），执行入口接受同序类型化参数（原生 `Value`）；参数个数/类型错误矩阵、NULL 三值语义与既有字面量类型门同源；prepare 一次、多次带参执行的预编译语句，schema 变化后失效契约按设计定稿（默认透明重计划）；不以字符串替换实现绑定。
3. **Iteration 划分随 Phase 2 调查定稿**（候选形态：类型化结果基建与调用面迁移 → 占位符与参数执行 → 预编译语句与失效契约）。

## Non-goals

（来自 tasks.md MS31 与 I088/I089 关联条目）

- PG Extended Query 协议（I071 触发锁定维持）；流式游标/背压（I065 long-term）；crate 边界拆分（I074 长期方向，不以本 change 为前置）。
- 查询取消、超时与事务状态契约（I090 open，待语义调查）。
- 执行器批量接口与查询内存预算（MS32）；Value 表示层改动（I020）；分配器评估（I066）。
- 多套占位符方言并行（仅一套位置式语法，不提供 `$N` 命名式公共面）。

## 默认假设（集中决策项——A1/A2 为 2026-10-21 用户裁定，其余取默认值闭合，非 TBD；用户审计可调整）

- **A1 单 change 结构（用户裁定）**：MS31 两任务一个 change、多 Iteration；T01 结果面先行、T02 在其上消费。覆盖 tasks.md workload「约 2 个 change（T01 一 change；T02 视 Plan 调查可 1-2）」注记。
- **A2 旧签名替换（用户裁定）**：`execute_sql`/`execute_in_tx` 改为类型化返回，不保留旧签名兼容层；CLI/Server/src 调用面与 tests 81 文件机械迁移；输出行为零变化由边界转换与既有测试零回归保证。
- **A3 错误通道形态**：类型化入口以 `Result` 承载语句错误，错误载荷携带消息与 SQLSTATE 分类（与既有 `Response::Error` 同源同分类）；具体错误类型形态随设计定稿。
- **A4 列元数据边界**：列名取既有计划输出列单源；列类型——直接列引用取声明类型、表达式列按推导矩阵（与 CTAS `infer_ctas_column_types` 对齐）；推导不可行处的表示随设计定稿（诚实处理，不虚构类型）。
- **A5 预编译失效契约**：schema 变化后预编译语句透明重计划、结果与直接执行一致（默认；Phase 2 调查发现结构性障碍时改为明确错误并在设计定稿）。
- **A6 占位符语法**：位置式 `?`，参数按出现序绑定；sqlparser 0.44 解析能力与值位置覆盖面（表达式/INSERT VALUES/UPDATE SET/WHERE 谓词）为 Phase 2 调查项，不可达位置诚实点名拒绝。
- **A7 并发与事务形态**：预编译语句与 `Database` 同为 `&self` 共享安全；参数化执行覆盖隐式与显式事务（`execute_in_tx` 对应形态）；不引入新隔离语义。
