## Purpose

记录有证据但尚未承诺实施的改进机会。条目使用 `Ixx` 编号，按 Phase 或主题分类。批准实施后创建 OpenSpec change 并标记 `promoted`。

## Requirements

### Requirement: 改进项可评估

改进项 SHALL 包含分类、问题、证据、影响、建议和状态。

#### Scenario: 发现未排期问题

- **WHEN** 已有证据表明存在改进机会但尚未批准实施
- **THEN** 使用递增 I 编号记录

#### Scenario: 批准实施

- **WHEN** 用户批准实施改进项
- **THEN** 创建 OpenSpec change 并把原条目标记 promoted

---

## Phase 1 基础设施（已完成）

**I001-I003 全部完成**（M41/M30/M38，详见 R08-013 + K14-16 + D09-10）

## Phase 2 存储引擎核心（已完成）

**I004-I008 全部完成**（M20/M19/M21/M36/M19，详见 K17-19, K22, D11-12, R10-013）

## Phase 3 并发控制

## I012: M42 消息传递重构

- **分类**: 架构 / 可维护性
- **问题**: 多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**: WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
- **依赖**: M32（I011）
- **状态**: planned（P3）
- **Legacy**: O012

## Phase 4 上层功能

## I014: M24 多隔离级别

- **分类**: 功能 / SQL 标准
- **问题**: 只有 Repeatable Read
- **方案**: Read Committed + Serializable（SSI）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O014

## I015: M25 多 Join 算法

- **分类**: 功能 / 查询优化
- **问题**: 只有 Hash Join
- **方案**: NLJ + SMJ + 启发式选择
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O015

## I016: M26 代价模型 + Join 重排

- **分类**: 功能 / 优化器
- **问题**: 固定 join 顺序，无 cardinality/selectivity
- **方案**: `TableStatistics` + `CostEstimator` + DP/贪心重排
- **依赖**: M25（I015）
- **状态**: planned（P4）
- **Legacy**: O016

## I017: M27 关联子查询缓存

- **分类**: 性能 / 子查询
- **问题**: 每行外层重新执行子查询
- **方案**: `SubqueryCache` 参数值→结果集 LRU
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O017

## I018: M28 多层关联子查询

⚠️ STALE [2026-09-09] — 建议在 30 天内确认、更新或归档（无路线图归属，规划时决断）

- **分类**: 功能 / 子查询
- **问题**: 显式拒绝多层嵌套
- **方案**: 递归遍历 + 多层注入
- **依赖**: M27（I017）
- **状态**: planned（P4）
- **Legacy**: O018

## I020: M37 clone 消除 Arc/Cow

- **分类**: 性能 / 分配
- **问题**: `Value::clone()` 在聚合/排序/JOIN 中反复调用
- **方案**: `Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O020

## I021: M39 INSERT 批量执行

⚠️ STALE [2026-09-09] — 建议在 30 天内确认、更新或归档（无路线图归属，规划时决断）

- **分类**: 性能 / 写入
- **问题**: 多值 INSERT 逐行执行
- **方案**: `bulk_insert(keys)` + `append_batch(records)`
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O021

## Phase 5 高级优化

## I024: M23 Varint Key 编码

- **分类**: 性能 / 存储
- **问题**: 固定 32B Key，INT PK 浪费 ~28B
- **方案**: `Key` 内部 `Vec<u8>` 变长编码
- **预期**: 索引空间 ~70% 缩减
- **依赖**: 无
- **状态**: planned（P5）
- **Legacy**: O024, D02 successor

## I025: M33 B+Tree 节点级锁

- **分类**: 并发 / 锁
- **问题**: 每次操作 lock 整棵树
- **方案**: Semaphore 限流 + latch coupling（crabbing protocol）
- **依赖**: M23（I024）
- **状态**: planned（P5）
- **Legacy**: O025

## I026: M35 脏页 writev 批量写回

- **分类**: 性能 / Checkpoint
- **问题**: 逐页 `write_at()` + 单独 `fsync()`
- **方案**: 连续页合并写 + `writev()` 向量化
- **预期**: Checkpoint 5-10x
- **依赖**: M31（已完成）+ M48（I013）
- **状态**: planned（P5）
- **Legacy**: O026, D12 下游

## I027: M43 并行扫描

⚠️ STALE [2026-09-09] — 建议在 30 天内确认、更新或归档（无路线图归属，规划时决断）

- **分类**: 性能 / 并行
- **问题**: 全表扫描单线程
- **方案**: 按页范围分区 + `mpsc` 汇聚
- **依赖**: M19（已完成）+ M22（I023）
- **状态**: planned（P5）
- **Legacy**: O027

## I028: M45 io_uring 批量提交

- **分类**: 性能 / I/O
- **问题**: `tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **方案**: `tokio-uring` 批量提交（IOSQE_IO_LINK）
- **预期**: I/O 延迟 -30-50%
- **依赖**: Linux 5.1+
- **状态**: planned（P5）
- **Legacy**: O028, K36

## 长期方向（未规划具体里程碑）

## I029: M46 瘦内部节点

- **分类**: 性能 / 存储
- **问题**: B+Tree 内部节点只存 separator keys，不存完整 Key
- **难度**: 高，需重构 B+Tree 分裂/合并逻辑
- **依赖**: M23（I024）
- **状态**: long-term
- **Legacy**: O029

## I030: M47 合并 Tag byte

- **分类**: 性能 / 序列化
- **问题**: Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
- **难度**: 低，但影响序列化格式兼容性
- **状态**: long-term
- **Legacy**: O030

## I031: B-Tree 撕裂树运行期根修（结构感知刷盘/驱逐改造）

- **分类**: 正确性 / 存储引擎
- **问题**: 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 含洞（父页指向未刷盘子页）与孤儿页（裸读实测 `scan_all` 184/10000 + `InvalidPageType` 洞页）。恢复侧已由 `redo_count > 0` 时索引去信任 + 重放后重建消解（MS10-T02 R7/R8），但运行期检查点间的磁盘树仍处于撕裂状态；MS12 整库加密若引入页级 transform 将放大对磁盘树一致性的依赖
- **候选方案**: 结构感知刷盘（子树后序）/ checkpoint 树快照 / no-steal 驱逐——均为 BufferPool 驱逐策略重设计，MS10-T02 design D10 拒绝并入
- **量化支撑**: D10 恢复重建代价实测 40k 行→8.86s、160k 行→40.7s（100 页池随机访存主导；撕裂树运行期根修可同时压缩该恢复代价）
- **状态**: planned（与 MS08-T03 脏页 writev 同域，实施前先量化）

## I032: `BufferPool::mark_tx_aborted` 空实现补全

- **分类**: 正确性 / 事务恢复
- **问题**: `mark_tx_aborted` 为 no-op（`buffer_pool.rs:369-371`）——`RecoveryManager::full_recover` 的 mark-uncommitted-aborted 步骤实际空转；未提交行仅靠 header `commit_tx_id=None` 的不可见性兜底，aborted 行物理滞留数据页
- **影响**: 当前语义自洽（未提交行不可见、重建谓词按 committed 排除），但未提交事务的页空间不可回收，且未来依赖「aborted 标记」的机制（如空间回收、更细的可见性）将踩空
- **方案**: 恢复期对 uncommitted 事务的行打显式 aborted 标记（header 扩展或墓碑化），或明确文档化「无标记」模型
- **状态**: planned（小改动，随下次触碰 transaction/recovery 面顺带评估）

## I033: update→delete 行旧版本在无快照扫描重现

- **分类**: 正确性 / 执行器
- **问题**: 版本链 T→A（update）→ tombstone（delete）中，墓碑替代者不抑制前驱（`superseder_suppresses` 对 `is_deleted` 恒 false，MS10-T02 R6 精确保留既有语义）→ 无快照扫描产出已删除行的旧版本 T
- **影响**: 语义为「既有行为未扩大」（R6 前同样重现）；MS10-T02 验收夹具 UPDATE/DELETE 域不相交故未触发；未来混合负载计数会虚高
- **方案**: 抑制谓词区分「已提交墓碑」（应抑制整条链）与「未提交墓碑」（不抑制、回溯前驱）——需对照 WAL committed 集合或 header 编码扩展
- **状态**: planned（与 MS09-T01 隔离级别工作同域，届时一并处理）

## I034: 裸 DataScan 子集投影的 CLI 表头返回全 schema

- **分类**: 正确性 / CLI 渲染
- **问题**: 无 WHERE 的裸 DataScan 子集投影，CLI 表头经 `get_plan_output_columns`（plan 节点 columns 元数据）返回全 schema 而行已按 projection 裁剪——`SELECT name FROM t` 表头 `["id","name"]`、行 `[["Alice"]]`；带 WHERE 的 IndexScan 路径表头正确（`["name"]`）。spec `cli-noninteractive-shell` R6 S1「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足
- **证据**: MS10-T04 Plan Review finding 5 独立探针复现（2026-09-09，revision `8827700`）；`tests/projection_test.rs`（MS10-T01）只断言 lib 行形状，未覆盖 CLI 表头
- **影响**: json 输出 `columns` 与 `rows` 字段数不一致，机器消费需二次裁剪；表格输出表头错位
- **方案**: `get_plan_output_columns` 对裸 DataScan 节点按 `projection` 索引裁剪表头；修正后同步校准 `tests/cli_test.rs::test_multi_statement_sequential_render` 的注释登记
- **状态**: planned

## I035: no-FROM SELECT 不受支持

- **分类**: 功能 / SQL 能力边界
- **问题**: `SELECT 1`（无 FROM 子句）在 plan 阶段报 `Plan error: Missing required field: FROM clause`（exit 3）——MS10-T04 S5 分号边界见证原拟 SQL 因此不可达，修订为含 FROM 等价 SQL（用户批准，见归档 change Blocker Handoff/Resolution）；`tests/cli_test.rs:481-482` 既有注释同证
- **证据**: MS10-T04 Act Blocker Handoff + Plan Review 独立复现（2026-09-09，revision `5c42ec8`）
- **影响**: 常量表达式查询、`SELECT current_setting()` 类无表探测不可达；agent/脚本日常探测用法受挫
- **方案**: planner 增加 no-FROM SELECT 臂（单行虚拟输入），属引擎能力扩展，需独立 change
- **状态**: planned

## I036: planner 简单 PK 等值 + 非 Int 字面量路由 Filter(Scan) 对无键行不可达

- **分类**: 正确性 / planner 路由
- **问题**: `WHERE <键列> = <非 Int 字面量>`（如 String 首列隐式 PK 表的 `WHERE s = 'x'`）经 `has_pk_equality` 结构化判定（不问可键控性）生成 Filter(Scan)，`ScanExecutor` 走 `index_manager.scan_all()` 索引遍历——键位不可键控（NULL/非 Int）的行落库不入索引（MS10-T05 001-rework 语义），经该 WHERE 形态不可达；无键行仅经非 PK 谓词或无 WHERE 的 DataScan 路径可见
- **证据**: MS10-T05 Iteration 001 001-rework Act Deviation 1 真二进制探针 + Plan Review 独立核实 `src/parser/planner/query.rs:430-475`（2026-09-09，归档 change `2026-09-09-2026-09-09-ms10-t05-lifecycle-subcommands`）；MS11-T03 双重实证同一根因的全空结果形态（2026-09-11，归档 change `2026-09-10-ms11-t03-scalar-functions`——无声明 PK 表首列字符串 Eq 经 `has_pk_equality`→Filter(Scan) 回退返回空行集 exit 0，pristine master 源码级探针 Eq→rows:[] vs Ne/Gt 正常 + CLI 探针，Plan Review 独立复现；测试以声明 PK 表形规避并注记）
- **影响**: 对含无键行的表按键位等值过滤漏行（静默不完整结果）；此前该形态表恒为空、行为不可观察
- **方案**: planner 对键位等值 + 不可键控字面量回退 DataScan 行内过滤（`to_key()==None` 时禁用索引路由）；属 planner 路由面，需独立 change
- **状态**: planned

## I037: UPDATE 键位为无键值后运行期旧键索引条目指向无键版本

- **分类**: 正确性 / update 执行器索引维护
- **问题**: 键行经 `UPDATE SET <键列> = NULL`（或不可键控值）后，`UpdateExecutor` 以旧 key 无条件 `index_manager.update(&self.key, new_row_id)`（`src/executor/update.rs:130-133`）——旧键条目指向键位已为 NULL 的版本：运行期对该键值 INSERT 被 DuplicateKey 误拒（无行实际持有该键值）、点查可达性语义含混；崩溃恢复后重建自然清除（无键版本不入重建索引），运行期与恢复后两态不一致
- **证据**: MS10-T05 Iteration 001 001-rework Plan Review Finding 5 代码核实 + `tests/keyless_row_test.rs` 恢复面行为（2026-09-09，归档 change 同上）；修复前该形态 Update 重放 RedoFailed（库不可打开），001-rework 后恢复面正确
- **影响**: 运行期唯一性检查误拒；运行期/恢复后索引内容不一致（重开自愈）
- **方案**: update 执行器对新值 `to_key()==None` 改为删除旧键条目（`index_manager.delete`）而非 update；行为变化需回归 `tests/keyless_row_test.rs` 与恢复套件
- **状态**: planned

## I038: GC 对无键行版本链不可达（gc_table scan_all 盲区）

- **分类**: 资源 / GC 覆盖面
- **问题**: `TableMeta::gc_table` 经 `index_manager.scan_all()` 枚举版本链，键位不可键控的行（MS10-T05 001-rework 起落库不入索引）不在索引中——其旧版本链永不被 GC 清理
- **证据**: MS10-T05 Iteration 001 001-rework Plan Context Risks 预判 + 实施后语义成立（2026-09-09，归档 change 同上）；`gc_table` 为可选维护路径（M10）
- **影响**: 含无键行的表长期频繁 UPDATE 场景下旧版本空间不回收；无正确性影响
- **方案**: GC 增加数据页链全扫模式（不经索引）或无键链登记结构；需评估成本后独立 change
- **状态**: planned

## I039: 多代 dump/restore 表名引号膨胀（ObjectName Display 即表名）

- **分类**: 正确性 / dump 保真
- **问题**: 引擎以 ObjectName 的 Display 形式为表名（`pipeline.rs:950/963`、`query.rs:101`、`ddl_dml.rs:287`）——带引号 DDL restore 重建后表名含引号字符，对该库再 dump 出现引号膨胀（`"items"` → `"""items"""`）；一代往返数据等价（`test_dump_restore_roundtrip_full_shape` 锁定），多代往返表名不保真；schema 命令对带引号建表同病
- **证据**: MS10-T05 Iteration 001 000-initial Act Deviation 1 探针 + Remaining Issues #2（2026-09-09，归档 change 同上）
- **影响**: 多代 dump→restore 链路表名逐代膨胀；Display 安全表名（CLI 常规路径）不受影响
- **方案**: 表名解析侧归一化（去引号）或 DDL 生成侧条件引号；涉引擎表名语义，需独立调查
- **状态**: planned

## I040: 负数字面量 INSERT 不可达（UnaryOp → UnsupportedValue）

- **分类**: 功能 / SQL 能力边界
- **问题**: planner `extract_insert_values` 只接受 `Expr::Value`/裸 NULL，负数被 sqlparser 解析为 UnaryOp → `UnsupportedValue`（`src/parser/planner/ddl_dml.rs:119`）——`INSERT INTO t VALUES (-1, ...)` 不可达；import/restore 对含负数的 CSV/dump 文本 exit 3 响亮失败（非静默）
- **证据**: MS10-T05 Iteration 001 000-initial Act Deviation 2 + Remaining Issues #3（2026-09-09，归档 change 同上）；`sql_literal` 对负数的渲染由 lib 单测锁定（导出侧正确、导入侧受限）
- **影响**: 负数数据无法经 SQL 面入库；dump 含负数文本的库无法 restore
- **方案**: `extract_insert_values` 接受 `UnaryOp::Neg(Value)` 折叠为负值；SQL 语义扩展，需独立 change
- **状态**: promoted（2026-09-10 并入 MS11-T01 实施——`extract_insert_values` +`UnaryOp{Minus, Value}` 臂，dump/restore/import 同函数自动受益；change 归档 `openspec/changes/archive/2026-09-10-ms11-t01-sql-expressions/`，spec `sql-expression-evaluation` R5）

## I041: cli::resolve 两个 env 测试并发改写进程全局 HOME/RTSQL_HOME 竞态

- **分类**: 稳定性 / 测试基建
- **问题**: `src/cli/resolve.rs` 的 `test_db_dir_env_cases` 与 `test_bare_name_env_cases` 为两个并发运行、各自持有独立 `EnvGuard` 改写进程全局 `HOME`/`RTSQL_HOME` 的测试（`src/cli/resolve.rs:79-127`）——guard drop 恢复 HOME 的窗口可撞上对方断言窗口，全量 `cargo test` 偶发失败（观察约 6 次全量 1 次）；单独 `--lib` 稳定通过
- **证据**: MS11-T02 Iteration 001 Act Remaining Issue #1（2026-09-10，change `2026-09-10-ms11-t02-sql-transaction-statements`）；新增 16 个子进程型 e2e 测试提高全量并行负载放大该既有窗口；Plan Review 独立全量复跑未复现（偶发）；本 change 未触碰 resolve.rs 且 diff 无 env 变更
- **影响**: 全量回归假失败（重跑即绿），干扰 CI/收尾判定；无产品影响
- **方案**: 两 env 用例合并为单测试顺序执行，或 env 用例串行化（同一测试线程内执行）；属测试基建修改，需独立小 change
- **状态**: planned

## I042: 边界子句拒绝 × 活跃会话事务组合路径无 e2e 锁定

- **分类**: 测试覆盖 / CLI 会话
- **问题**: spec `sql-transaction-statements` R2 各拒绝场景均为独立调用、空闲会话；「会话事务活跃中遇边界子句」（如 `BEGIN; INSERT ...; SAVEPOINT sp1`）的组合路径无 spec 场景与 e2e 用例——该路径行为由 `run_sql` Err 分支 + D5 收尾规则组合产生（拒绝不改变会话态、回滚属收尾），已由 Plan Review 二进制探针验证正确（exit 3 + D3 点名文案 + 事务上下文后缀 + 重开无残留）但无回归锁定
- **证据**: MS11-T02 Plan Review Finding F3 探针（2026-09-10，change `2026-09-10-ms11-t02-sql-transaction-statements`，iterations/001-cli-session/000-initial.md）
- **影响**: 未来重构 `run_sql` 错误路径时该组合语义可能静默回归；无当前正确性问题
- **方案**: `tests/tx_statement_test.rs` 增加组合场景 e2e（可选测试加固，随下一次触碰 CLI 会话面的 change 顺带实施即可）
- **状态**: planned

## I043: 标量函数极端输入边界（abs i64::MIN 溢出 / round 极端 digits 非有限值）

- **分类**: 健壮性 / 标量函数
- **问题**: `abs(-9223372036854775808)`（i64::MIN）按契约实现为 `i64::abs()`（`src/executor/function.rs` ABS 臂），debug 构建溢出 panic、release 回绕为负；`round(x, digits)` 按 `10f64.powi(digits)` 乘除实现，|digits|>308 时 factor 溢出为 inf/0，结果 inf/NaN（SQLite `round(1,1000)=1.0`）。spec `sql-scalar-functions` R3 仅锁定常规值（`abs(-5)`/`round(123.4,-1)` 等），两边界均不在锁定面
- **证据**: MS11-T03 Iteration 001 Act Remaining Issues #1/#2（2026-09-11，归档 change `2026-09-10-ms11-t03-scalar-functions`；Plan Review 独立核实实现契约与公式）
- **影响**: 极端输入下 panic/非有限值；常规分析负载不可达，无当前正确性问题
- **方案**: abs 改 `checked_abs` 显式溢出报 `ValueError` 或文档化回绕语义；round 对 |digits| 设上限截断或文档化——随下次触碰 `function.rs` 的 change 顺带评估，需用户裁定方向
- **状态**: planned

## I044: 标量函数名大小写不敏感缺 SQL 层测试见证

- **分类**: 测试覆盖 / SQL 函数层
- **问题**: spec `sql-scalar-functions` R1「函数名匹配 SHALL 大小写不敏感」无 SQL 层 e2e 见证——`tests/` 全部使用小写形态，无 `UPPER(...)`/`Abs(...)` 等大写/混合变体用例（tests/ 全目录 grep 实证）；实现层成立（`ast.rs` 两放行门与 planner 函数臂均先 `to_uppercase()` 规范化再查注册表，Plan Review 代码核实），但该 SHALL 子句在 23 场景集与两轮 RTM 中均无对应场景
- **证据**: MS11-T03 Plan Review Finding F1（2026-09-11，归档 change 同上；大写变体 grep + 两处规范化点核实）
- **影响**: 未来重构函数名匹配路径时大小写语义可能静默回归；无当前正确性问题
- **方案**: `tests/scalar_function_test.rs` 增加大写/混合大小写变体用例（可选测试加固，随下次触碰函数面的 change 顺带实施）
- **状态**: planned

## I045: 主 specs Purpose 占位与 TBD/TODO 残留（validate --specs 持续 WARNING）

- **分类**: 文档质量 / OpenSpec 语料库
- **问题**: 多个主 spec 的 `## Purpose` 仍为 `openspec archive` 自动写入的占位句或含 TBD/TODO 标记（grep 实证 10 文件：database-file-format-header、planner-module-decomposition、dml-transaction-lifecycle、drop-table-physical-free、wal-recovery-replay-integrity、pipeline-stage-decomposition、wal-writer-handle-reuse、database-file-lock、cli-noninteractive-shell、storage-io-optimization 等）——`openspec validate --specs` 持续 WARNING（passed/failed 不受影响）；2026-09-11 起新 spec `sql-scalar-functions` 已补真实 Purpose，不再新增占位
- **证据**: MS11-T03 Iteration 001 Act Remaining Issue #3 + Plan Review 复跑 validate 输出（2026-09-11，归档 change 同上）
- **影响**: 语料库能力入口可读性下降；validate 输出噪声持续
- **方案**: 逐 spec 补写真实 Purpose（一句能力定位 + 来源 change 引用，格式对齐 `sql-transaction-statements`/`sql-scalar-functions` 先例）；纯文档工作，可一次性小 change 或随下一次 docs 收尾顺带
- **状态**: planned

<!-- arc: ARC-202609092322 --> 7 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
