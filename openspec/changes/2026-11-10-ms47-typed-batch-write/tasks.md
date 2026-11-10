# MS47 类型化批量写入入口的去留

来源：`.agents/docs/tasks.md` 的 MS47，承接 I021。约束：本次每次测试调用带 `--test-threads=1`，多个测试目标分成多次命令顺序执行。本变更不安排全量，也不跑全量。

Gate 1：PASS。需求、场景和范围来自 MS47 与本次目标里写死的 10% 采纳条件。这是批准，不是跳过。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格增量在 `specs/typed-batch-write/spec.md`。
- 000：当前轮。目录 `iterations/000-measure-and-decide/`，Cycle `000-initial.md`。只测量并裁定。
- 001：未展开。仅当 000 的裁定为达到 10% 时才建目录。低于 10% 时不展开，变更以「不建入口」收束。
- Persisted Evidence：none。
- 跳过项：无。全量不在本变更预算内，理由写在每个 Iteration 的 Verification boundary，不记成 Gate 跳过。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 100 条插入的诊断时间可打印，行可见，产品代码不变 | pending |
| T2 | 000 | T1 | 用 T1 的整数裁定建入口或不建入口，并写入执行记录 | pending |
| T3 | 001 | T2 的裁定为达到 10% | `insert_batch` 与逐行插入的检查、索引和整批回滚一致 | 未展开 |

## Iteration Plan

### Iteration 000: 测量占比并裁定入口

- Tasks: T1, T2
- Depends on: None
- Stable result: 两个纳秒数和一次裁定。达到 10% 时，下一轮可以依赖「需要入口」这个结论。低于 10% 时，变更的稳定结果是不新增入口。
- Verification boundary: 本轮只跑 `tests/typed_batch_write_test.rs` 里的测量测试，一次，release，`--test-threads=1`。不跑全量。跳过本变更收尾全量的理由是用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。风险：本变更收尾前没有全量；合回主分支后那一次失败时，排查范围包含 MS46 与 MS47。
- Diagnostic boundary: 这一轮的失败只看这条测试的摘要阶段、打印的整数和 100 行是否可见。不把导入、CSV、索引格式或另一个变更的改动纳入排查。
- Non-goals: 不实现 `insert_batch`。不改 `execute_sql`。不展开 Iteration 001。

平衡：本轮的稳定结果是采纳裁定，后续实现与它不是同一个故障域。只有一条测试，但它决定入口是否存在，不并进实现轮。实现轮在数字出来之前不是本轮的工作。

### Iteration 001: 类型化批量写入入口

- Tasks: T3
- Depends on: Iteration 000 的裁定为 `execute_exclusive_ns * 10 >= total_ns`
- Stable result: `Database::insert_batch` 可用。类型检查、约束、索引维护和失败回滚与逐行插入一致。整批失败则整批回滚。`execute_sql` 插入保持原行为。
- Verification boundary: 顺序执行下面两条，都带 `--test-threads=1`，不并行。第一条是本变更的测试文件，第二条是已有的单条插入诊断测试。不跑全量。跳过本变更收尾全量的理由是用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。风险：本变更收尾前没有全量；合回主分支后那一次失败时，排查范围包含 MS46 与 MS47。
- Diagnostic boundary: `insert_batch`、`execute_stage` 的插入臂、`InsertExecutor`。导入循环、CSV 读取、分配器和刷盘策略不在排查范围。
- Non-goals: 不改 `src/cli/lifecycle.rs`。不把 import 或 CSV 接到入口上。不增加部分成功、列清单、`ON CONFLICT` 或显式事务参数。不新增诊断阶段。不新增放弃 future 的暂停夹具。

平衡：入口的成功、拒绝和回滚是同一个写路径上的一个可验证结果。拆成多轮会把同一次 `execute_stage` 调用切到不同的排查范围里。本轮未展开；000 未达标时保持不展开。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 一百条逐行插入的占比用诊断阶段测量 | T1 | `row_insert_share_of_hundred_inserts` | Covered |
| 达到百分之十才提供批量入口 | T3 | `insert_batch_rolls_back_when_later_row_fails` | Covered |
| 低于百分之十时不新增入口 | T2 | `row_insert_share_of_hundred_inserts` 的输出行 | Covered |

没有 Simplified。没有 Missing。

## T1 与 T2

执行依据是 `iterations/000-measure-and-decide/000-initial.md` 的 Task Definition。本文件不另写一套步骤。

## T3: 实现 insert_batch

本任务只在 Iteration 001 展开之后执行。000 的 Cycle 不执行本任务。

- Requirement/Scenario: `typed-batch-write` 的「达到百分之十才提供批量入口」及其全部场景。
- Depends on: T2 记录的整数满足 `execute_exclusive_ns * 10 >= total_ns`。
- Targets: `src/database.rs` 的 `Database::insert_batch`；调用 `src/pipeline.rs` 的 `execute_stage`；复用 `src/executor/insert.rs` 的 `InsertExecutor`，不复制其循环。测试在 `tests/typed_batch_write_test.rs`。
- Current behavior: 没有 `insert_batch`。多行 `INSERT` 经 `execute_stage` 在一个隐式事务里逐行插入，失败则 `abort`。缺表、视图和行宽在 `begin` 前拒绝。`execute_sql` 不加入显式事务。
- Required behavior: 方法签名、检查顺序、错误文本、SQLSTATE、空批次、显式 NULL、整批回滚和显式事务隔离与 `specs/typed-batch-write/spec.md` 的第二条 requirement 一致。对照物是同一数据库形状上的 `execute_sql`，不是另写一套错误字符串。非空且通过前置检查的调用使用 `execute_stage(..., profiling = false)`。
- Preserve: `execute_sql` 的插入结果、错误和诊断阶段。`src/cli/lifecycle.rs` 的逐行导入。文件格式。`InsertExecutor` 的门序。
- Forbidden: 部分成功；列清单和 `ON CONFLICT`；在 `database.rs` 重写类型门或索引维护；行宽不符仍进入 `InsertExecutor`；把 import 或 CSV 改到本入口；新增诊断阶段名、判定脚本或身份字段；修改 `openspec/specs/` 里的已有规格正文。
- First test: 在 `tests/typed_batch_write_test.rs` 增加 `insert_batch_rolls_back_when_later_row_fails`。方法尚不存在时，`cargo test --offline --test typed_batch_write_test insert_batch_rolls_back_when_later_row_fails -- --test-threads=1` 应编译失败或断言失败。同时写上下列测试，都在这个文件里：成功路径与二级索引对照、类型不符、`NOT NULL`、`UNIQUE`、缺表、视图、行宽、空批次、声明 DEFAULT 下列上的显式 NULL、显式事务回滚后行仍在、只读会话的非空拒绝和空切片返回 0、内存库上的成功与失败回滚。每项失败路径都和同值 `execute_sql` 比较错误文本、SQLSTATE 和表内容。
- GREEN condition: `cargo test --offline --test typed_batch_write_test -- --test-threads=1` 退出码 0，failed 为 0。其中测量测试仍然通过。
- Verification: 上一条命令通过后，再单独执行 `cargo test --offline --test diagnostics_statement_test insert_commit_is_child_of_execute -- --test-threads=1`，退出码 0。两条命令顺序执行，不合并成一次 cargo 调用。失败表示入口改变了既有插入的诊断阶段，或新入口与 `execute_sql` 不一致。
- Stop when: 为了和 `execute_sql` 一致必须改成部分成功、必须改 `InsertExecutor` 的门序，或不能经 `execute_stage` 完成隐式事务时，停止并回到计划。不在本任务里改导入。

## 验证命令汇总

000 的命令只在 Cycle 文件里。001 的两条命令见 T3。全部带 `--test-threads=1`。没有全量命令。
