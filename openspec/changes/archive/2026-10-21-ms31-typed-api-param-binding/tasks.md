# Tasks: MS31 类型化库结果与参数绑定 API

来源：proposal What Changes 1/2（T01 + T02）；设计依据 design.md D1-D8。

> Status: 交付完成待收尾（2026-10-22：Iteration 000 完成——Cycle `000-initial` 修复轮后复审 **accepted**〔G1 benches 编译面 / G2 声明来源一致性两 Gap 关闭，F2 前提复核裁定 Act 判定成立〕；类型化入口（`Result<SqlOutcome, SqlError>`）+ 列类型装配落地，核心路径零 `network::protocol` 依赖，全量终态 2081/0/2。Iteration 001 Cycle `000-initial` 已实施并复审 **accepted**（2026-10-22：T5–T8 落地——参数化通道 `execute_with_params`/`execute_in_tx_with_params` + 拒绝面 + 计划面编译，全量终态 2109/0/2；设计回折 D5/D6 已裁定回写）。Iteration 002 Cycle `000-initial` 已实施并复审 **accepted**（2026-10-23：T9–T11 落地——schema 版本戳单点 + `prepare`/`PreparedStatement` + 失效/并发/事务矩阵，全量终态 2123/0/2；D7 并发机制设计回折经 Review 裁定接受并回写 design.md——执行持计划副本 + 同语句并发经闸门串行化）。三个 Iteration 全部 accepted，T1-T11 全数完成；收尾归档待用户指令）

## 任务清单

| Task | 目标 | Iteration |
|---|---|---|
| T1 | 公共结果类型与边界转换：`src/result.rs`（SqlOutcome/ColumnMeta/SqlError + value_to_json 迁入）+ `network::protocol::to_response` | 000 |
| T2 | 入口签名替换与全调用面迁移：pipeline 全链 Result 化、database.rs 签名、handler/lifecycle/integrity/cli 主路径、tests 81 文件（types 先 names + None） | 000 |
| T3 | 列类型装配：`plan_output_types` 声明类型 walk + 结果行值推导兜底 | 000 |
| T4 | 全量分批回归（M19）与 CLI/协议输出零变化终验 | 000 |
| T5 | AST 占位符计数与不可达位拒绝面：`count_placeholders`、`?N` 拒绝、无参个数错误、DML 键位/标识位/DDL 点名拒绝 | 001 |
| T6 | 计划面占位符编译：expression.rs Placeholder 臂（`__param_{i}`）、UPDATE SET 表达式路由、InsertNode/UpsertNode `param_slots` 与 ddl_dml 装配 | 001 |
| T7 | 参数执行通道：`execute_with_params`/`execute_in_tx_with_params`（校验→plan→注入→物化→执行）与错误/NULL/TEXT 真值矩阵 | 001 |
| T8 | 全量分批回归（M19）预算点 | 001 |
| T9 | schema 版本戳：`Database.schema_version` + `bump_schema_version` 单点收敛全部 `plan_cache.clear()` 站点 | 002 |
| T10 | prepare/PreparedStatement：prepare 一次、clone-注入-执行、版本不匹配透明 re-plan、`execute_in_tx`、`&self` 并发安全 | 002 |
| T11 | 失效/并发/事务测试矩阵 + 全量分批回归（M19）预算点 | 002 |

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| typed-library-results R1 类型化执行结果（四场景） | T2（载体/入口/错误）+ T3（列类型） | `tests/typed_api_test.rs`（T2 先行测试：原生值/列名/DML 行数/错误载荷；T3 增类型断言） | Covered |
| typed-library-results R2 核心/协议解耦 | T1（词表位置）+ T2（全链去依赖） | T1 单测（to_response 映射）+ T2 后 grep 断言（核心路径无 `network::protocol`） | Covered |
| typed-library-results R3 边界转换输出零变化 | T1（转换同源）+ T2（边界接线）+ T4（全量终验） | T1 单测（value_to_json 逐字节）+ T4 全量分批回归（既有断言零校准） | Covered |
| sql-parameter-binding R1 占位符与类型化参数执行（六场景，含 DML 键位拒绝——Phase 2 调查定稿修订，见 design D5/R4） | T5（计数/拒绝面）+ T6（计划面编译）+ T7（通道与矩阵） | `tests/param_binding_test.rs`（T5 拒绝矩阵 RED；T7 行为矩阵） | Covered |
| sql-parameter-binding R2 可复用预编译语句（四场景） | T9（版本戳）+ T10（prepare/execute）+ T11（失效/并发/事务矩阵） | `tests/prepared_statement_test.rs`（T10 先行测试） | Covered |

注：sql-parameter-binding「DML 定位键位占位符点名拒绝」场景为 Phase 2 调查后对 Gate 1 版本的修订（原场景文本含 `WHERE a = ?` 可达形态；调查证实 Key 为计划期物化不可达，`src/parser/planner/ddl_dml.rs:1976-1986`）——随 Gate 2 一并提请确认。

## Iteration Plan

### Iteration 000: 类型化结果基建与调用面迁移（T01）

- Status: completed（2026-10-22 复审 accepted——T1–T4 完成、Gate 4 通过、R-a/R-b 修复轮独立核对通过；全量终态 2081/0/2，核心路径零 network::protocol 依赖）
- Tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: lib 公共入口（`execute_sql`/`execute_in_tx` 及 stage 家族）返回 `Result<SqlOutcome, SqlError>`；核心路径（database/pipeline/executor/storage/transaction）无 `network::protocol` 依赖；CLI 四格式与 PG/JSON 协议输出逐字节零变化；全量测试零回归（81 文件迁移后断言原样通过）
- Verification boundary: T1/T3 单元与集成测试即时验证；T2 原子面（编译完整性迫使 src + tests 同轮）；昂贵验证——全量分批回归（M19）在本 Iteration 末预算一次（T4），CLI/协议字节等价由既有 e2e/协议测试全量回归承担（render/pg_protocol/序列化零改动的结构性保证 + 既有断言）
- Diagnostic boundary: `src/result.rs`、`src/pipeline.rs`（execute 链与 execute_executor/漏斗点）、`src/database.rs`、`src/network/protocol.rs`、`src/network/handler.rs`、`src/cli/mod.rs` 主路径、迁移后失败的测试文件
- Non-goals: 占位符与参数（Iteration 001）、预编译语句（Iteration 002）、列类型的表达式静态推导（值推导兜底已定稿，见 design D3/R3）

### Iteration 001: 占位符与类型化参数执行（T02 前半）

- Status: completed（2026-10-22 复审 accepted——T5–T8 完成、Gate 3/4 通过、偏差与设计回折裁定接受（design.md D5/D6 已回写）；全量终态 2109/0/2）
- Tasks: T5, T6, T7, T8
- Depends on: Iteration 000（消费 SqlOutcome/SqlError 形态与注入通道）
- Stable baseline: SQL 文本位置式 `?` 全可达位（SELECT 谓词/投影、UPDATE SET 右值、INSERT/UPSERT 值位）绑定原生 Value 参数；个数/类型错误矩阵与 NULL 三值语义锁定；不可达位（DML WHERE 键位、标识位、DDL、`?N`）点名拒绝
- Verification boundary: T5/T6 拒绝面与计划形状先行测试；T7 行为矩阵（等值/值位/个数/类型/NULL/TEXT 真值——`evaluate_ref` 借用通道限制锁定点，见 design D6）真实入口验证；全量分批回归（M19）预算一次（T8）
- Diagnostic boundary: `src/parser/planner/expression.rs`、`src/parser/planner/ddl_dml.rs`、`src/executor/plan.rs`（param_slots）、`src/pipeline.rs` 参数通道、`src/parser/` 计数 walk
- Non-goals: 预编译与失效契约（Iteration 002）、DML WHERE 键位参数（design R4——需执行期键求值重构，独立设计）、多套方言

### Iteration 002: 可复用预编译语句（T02 后半）

- Status: completed（2026-10-23 复审 accepted——T9–T11 完成、Gate 3/4 通过、D7 并发机制设计回折裁定接受〔执行持计划副本 + 同语句并发经闸门串行化，见 design D7 回折〕；全量终态 2123/0/2）
- Tasks: T9, T10, T11
- Depends on: Iteration 001（消费参数通道与注入/物化协议）
- Stable baseline: prepare 一次、多次带参执行；schema 变化（DDL 八类 + CTAS）后透明 re-plan、结果与直接执行一致；显式事务内执行；`&self` 并发安全（执行持计划副本，同一条语句的并发执行经闸门串行化——计划树参数节点为共享可变状态，机制定稿见 design D7 回折）
- Verification boundary: T9 版本戳单点测试；T10/T11 行为矩阵（复用/失效/事务/并发）真实入口验证；全量分批回归（M19）预算一次（T11）
- Diagnostic boundary: `src/database.rs`（schema_version/prepare 家族）、`src/pipeline.rs`（bump 单点与 re-plan 路径）
- Non-goals: plan cache key 语义重构、prepare 的 attach 路由面（lib 面与 execute_sql 一致）、PG Extended Query 协议
