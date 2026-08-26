# 任务清单：MS06-T03 + MS06-T04（WAL 持久句柄 + Pipeline 三阶段拆分）

> 关联里程碑：MS06-T03、MS06-T04（`.claude/docs/tasks.md` MS06 节剩余两项）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 Iteration：两个逻辑 Iteration——`000-wal-handle`（T1-T2）、`001-pipeline-stages`（T3-T4）
>
> 修订记录：2026-08-26 初版（Plan 创建）

## Iteration Plan

### Iteration 000: WAL 持久句柄复用

- Tasks: T1, T2
- Depends on: None
- Stable baseline: `WalWriter` 全部 5 个 IO 方法经单一句柄操作；10K tx 压测 fd 净增量 < 10 断言进 cargo test；错误/LSN 对外语义零变化；WAL 族回归测试全绿
- Verification boundary: `cargo test --test wal_handle_test` 全绿 + WAL 回归族（wal_writer/wal_buffer/checkpoint/recovery/recovery_e2e）全绿 + `cargo build` 无警告
- Diagnostic boundary: `src/wal/writer.rs` + `tests/wal_handle_test.rs`；调用方（buffer/checkpoint/database）只读不改
- Non-goals: 错误重试策略、LSN 语义变更、WAL 文件格式变更、writer task 化重构

### Iteration 001: Pipeline 三阶段拆分与可观测性

- Tasks: T3, T4
- Depends on: Iteration 000（仅顺序依赖，无代码耦合；000 归档基线后启动）
- Stable baseline: execute_inner 缩为编排器；parse_stage/plan_stage/execute_stage 三个 pub 函数可独立调用；三段顶层计时接入现有开关；阶段级单测 + 三阶段独立 bench 可运行；pipeline/dml_tx_id 等回归全绿
- Verification boundary: pipeline 单元测试全绿 + `cargo test --test pipeline_test --test dml_tx_id_test` 全绿 + `cargo bench --bench pipeline_stages_bench` 编译运行产出数据
- Diagnostic boundary: `src/pipeline.rs` + `src/profiling.rs`（如需微调计时接口）+ `tests/pipeline_test.rs`（只读回归）+ 新增 bench 文件
- Non-goals: 执行器行为变化、缓存策略变化、性能调优、错误消息文案变化

### 平衡审计

- 000 与 001 分属存储域/管道域，故障域与验证边界不同 → 拆分成立
- T1+T2 同 Iteration：句柄改造（T1）与其验收测试（T2）是同一可验证结果的两面，拆开则任一单独不成稳定基线
- T3+T4 同 Iteration：三阶段拆分（T3）必须伴随阶段单测与 bench（T4）才能证明"独立可测可基准"，同属一个验收闭环
- 每 Iteration 各 2 task，工作量适中，无过碎或过重

## Iteration 000 — WAL 持久句柄

### T1: WalWriter 单句柄改造（R1/R3/R4 → S1/S2/S3/S4/S5, S6/S7）✅

- [x] 1.1 `src/wal/writer.rs`：结构体新增 `file: Arc<std::sync::Mutex<std::fs::File>>` 字段；`open()` 打开 create+append+read 后保留句柄入字段
- [x] 1.2 `write_record`：spawn_blocking 内 lock → seek(End(0)) → stream_position → write_all(buf) → 返回 lsn；删除逐次 open
- [x] 1.3 `fsync`：lock → sync_all；删除逐次 open
- [x] 1.4 `truncate_to`：lock → set_len(lsn)；删除逐次 open
- [x] 1.5 `get_current_lsn`：lock → metadata().len()；删除逐次 open
- [x] 1.6 `write_batch`：lock → 逐条 serialize_with_lsn + write_all → sync_all；删除逐次 open
- [x] 1.7 全方法 IO 失败保持 `WalError::IoError(e.to_string())` 上抛，无重试无重开
- [x] 1.8 公开方法签名不变（`&self` async）；`wal_path` 字段保留用于诊断
- [x] 1.9 `cargo build` 通过且无新警告

### T2: fd 上界与行为见证测试（R2 及 R1/R3/R4 场景固化）✅

- [x] 2.1 新增 `tests/wal_handle_test.rs::test_fd_bound_under_10k_tx`：tempdir Database::open → CREATE TABLE → 10K 次 execute_sql(INSERT)；压测前后 `/proc/self/fd` read_dir 计数，净增量断言 < 10
- [x] 2.2 `test_write_record_lsn_equals_file_offset`：顺序写 ≥3 条，逐条断言返回 LSN == 写前文件长度、严格递增、首条 == 0
- [x] 2.3 `test_truncate_then_append_same_handle`：写 n 条 → truncate_to(中间位) → get_current_lsn 反映截断长度 → 再写一条落在新末尾且 LSN 正确
- [x] 2.4 `test_concurrent_writers_recovery_consistent`：≥4 任务并发共享 Arc<WalWriter> 各写多条 → drop writer → RecoveryManager/WalReader 完整解析全部记录无错误
- [x] 2.5 RED 先行确认：T1 完成前 2.1 在现状代码上应能暴露 fd churn（若现状偶然通过则记录实际计数作为对照证据，不阻塞）；T1 后全绿
- [x] 2.6 回归族全绿：`cargo test --test wal_writer_test --test wal_buffer_test --test checkpoint_test --test recovery_test --test recovery_e2e_test`（0 failed，数量与基线一致）
- [x] 2.7 `cargo test --test executor_test` 全绿（其 5 处 `WalWriter::open(":memory:")` setup 零修改通过）

## Iteration 001 — Pipeline 三阶段拆分

### T3: parse/plan/execute 三阶段函数与编排器（R5 → S8-S13）

- [x] 3.1 `src/pipeline.rs`：新增 `pub async fn parse_stage(sql: &str) -> Result<Vec<Statement>, String>`——封装 parse_sql 调用；Err 返回 "Parse error: {e}" / 空语句返回 "Empty SQL"
- [x] 3.2 新增 `pub async fn plan_stage(database, sql, stmt) -> Result<PhysicalPlan, String>`——DDL 变体走 PlanBuilder::new().build_plan；其余走 register_table + build_plan + is_cacheable→put；Err 返回 "Plan error: {}" 或 "Table '{}' not found: {}"（格式逐一保持）
- [x] 3.3 新增 `pub async fn execute_stage(database, plan) -> Response`——按 PhysicalPlan 变体路由：DDL 直包 Executor 且成功后 plan_cache.clear()（时序保持：执行成功后才清）；DML begin→prefetch abort meta→create_executor(tx_id)→失败 abort→执行→commit/abort（Commit failed/Abort failed 消息保持）；其余 create_executor(None)→执行
- [x] 3.4 `execute_inner` 重写为编排器：cache lookup（cache_hit_check 计时保持）→ 命中直接 execute_stage(cached) → 未命中 parse_stage→plan_stage→execute_stage → 各终止点 print_timings 保持
- [x] 3.5 删除原 cache-hit 早退重复块与内联三分支；`create_executor_from_plan`/`register_table`/`is_cacheable`/提取辅助族零修改复用
- [x] 3.6 profiling 三段顶层计时：编排器以 record_time("parse"/"plan"/"execute") 包裹各 stage 调用；stage 函数接收 `profiling: bool` 参数守卫内部子指标（table_metadata_lookup 等）；所有 record/print 严格处于 profiling 守卫下（task_local panic 约束）
- [x] 3.7 DML 事务包裹语义逐行保持（begin/commit/abort 调用序列与现状一致）；`dml_tx_id_test` 6 测试零修改通过

### T4: 阶段级单测 + 三阶段独立 micro-bench（R6/R7/R8 → S14/S15/S16）

- [x] 4.1 `src/pipeline.rs` 新增 `#[cfg(test)] mod tests`：
  - parse_stage：合法 SQL 产语句 / 非法 SQL "Parse error:" / 空串 "Empty SQL"
  - plan_stage：已建表 SELECT 产出扫描类计划 / 不存在表 Err 含 "not found" / SELECT 写入后 plan_cache_len()==1 且 INSERT 不增加
  - execute_stage：简单查询 plan 产正确 Response / DDL plan 执行成功后 cache 清空
- [x] 4.2 新增 `benches/pipeline_stages_bench.rs`：criterion + tokio Runtime + benches/common 模式；三组 benchmark 分别测量 parse_stage / plan_stage（防 cache hit 干扰）/ execute_stage（预热后跑预构建 plan）
- [x] 4.3 `Cargo.toml` 登记 `[[bench]] name = "pipeline_stages_bench"`（harness = false）
- [x] 4.4 `cargo bench --bench pipeline_stages_bench` 编译并产出三阶段测量数据（无数值阈值）
- [x] 4.5 回归全绿：`cargo test --test pipeline_test --test dml_tx_id_test --test plan_cache_test`（数量与基线一致）

## 全局验证门（两 Iteration 完成后）

- [ ] V1 `cargo build --all-targets` 通过无警告
- [ ] V2 `cargo clippy --all-targets -- -D warnings` 全绿
- [ ] V3 `cargo test` 全量：504 基线 + 新增测试全绿，0 failures

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 持久句柄复用 | S1-S5 | §2 | T1 | 000 | `wal/writer.rs::WalWriter` 5 方法 | wal_writer_test 回归 + wal_handle_test 2.2/2.3/2.4 | None | Covered |
| R2 fd 上界可验证 | S6(fd 断言) | §2 | T2 | 000 | `tests/wal_handle_test.rs::test_fd_bound_under_10k_tx` | 同左 | lsof→/proc/self/fd（G2 用户批准）| Covered |
| R3 错误语义保持 | S3(失败上抛) | §2 | T1 | 000 | `wal/writer.rs` 各方法错误路径 | wal_writer_test 既有断言 + 2.6 回归 | None | Covered |
| R4 LSN 文件位置语义 | S7(offset LSN) | §2 | T1/T2 | 000 | `write_record`/`write_batch` | wal_handle_test 2.2 + wal_buffer_test 回归 | None | Covered |
| R5 三阶段拆分 | S8-S13 | §3 | T3 | 001 | `pipeline.rs::{parse_stage,plan_stage,execute_stage,execute_inner}` | pipeline_test(17)+dml_tx_id_test(6) 回归 | None | Covered |
| R6 阶段级单测 | S14 | §3 | T4 | 001 | `pipeline.rs #[cfg(test)] mod tests` | 4.1 新增单测 | None | Covered |
| R7 三段顶层计时 | S15 | §3 | T3 | 001 | `execute_inner` 编排器 + `profiling.rs`（如需） | 手动 RTSQL_PROFILING 观测 + 单测守卫路径 | 输出名变更（G4 批准）| Covered |
| R8 独立 micro-bench | S16 | §3 | T4 | 001 | `benches/pipeline_stages_bench.rs` + Cargo.toml | `cargo bench --bench pipeline_stages_bench` | None | Covered |

Scenario 编号对应 delta specs：S1-S5 = wal-writer-handle-reuse；S6-S7 补充编号见 spec 内 Scenario 标题；S8-S16 = pipeline-stage-decomposition。
