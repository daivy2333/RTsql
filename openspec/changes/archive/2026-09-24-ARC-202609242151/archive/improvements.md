# improvements 归档全文（ARC-202609242151）

> 来源：`openspec/specs/improvements/spec.md` 2026-09-24 21:34 (+0800) 工作区状态，逐字抽取。
> 恢复：Maintainer 经本文件按原编号插回源文档对应 Phase 位置。

## I014: M24 多隔离级别

- **分类**: 功能 / SQL 标准
- **问题**: 只有 Repeatable Read
- **方案**: Read Committed + Serializable（SSI）
- **依赖**: 无
- **状态**: promoted（Read Committed 部分已于 MS09-T01 实施——`IsolationLevel` 枚举 + `Database::open_with_isolation`（默认 RR 逐字节等价）+ RC 语句级已提交视图，spec `transaction-isolation-levels`，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`；Serializable/SSI 维持非目标，未排期）
- **Legacy**: O014

## I017: M27 关联子查询缓存

- **分类**: 性能 / 子查询
- **问题**: 每行外层重新执行子查询
- **方案**: `SubqueryCache` 参数值→结果集 LRU
- **依赖**: 无
- **状态**: promoted（MS09-T04 实施——`SubqueryEval`/`SemiJoin`/`AntiJoin` 三执行器关联臂语句级缓存 `HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`（design D6 修正 DA1 的 LRU 设想：语句界有界 HashMap 足够），spec `correlated-subquery-cache`，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`）
- **Legacy**: O017

## I032: `BufferPool::mark_tx_aborted` 空实现补全

- **分类**: 正确性 / 事务恢复
- **问题**: `mark_tx_aborted` 为 no-op（`buffer_pool.rs:369-371`）——`RecoveryManager::full_recover` 的 mark-uncommitted-aborted 步骤实际空转；未提交行仅靠 header `commit_tx_id=None` 的不可见性兜底，aborted 行物理滞留数据页
- **影响**: 当前语义自洽（未提交行不可见、重建谓词按 committed 排除），但未提交事务的页空间不可回收，且未来依赖「aborted 标记」的机制（如空间回收、更细的可见性）将踩空
- **方案**: 恢复期对 uncommitted 事务的行打显式 aborted 标记（header 扩展或墓碑化），或明确文档化「无标记」模型
- **状态**: promoted（MS09-T01 实施——恢复期 `mark_uncommitted_aborted` 页链迭代显式中性化 + `BufferPool::mark_tx_aborted` no-op 移除，spec `mvcc-tombstone-visibility` R4，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`）

## I033: update→delete 行旧版本在无快照扫描重现

- **分类**: 正确性 / 执行器
- **问题**: 版本链 T→A（update）→ tombstone（delete）中，墓碑替代者不抑制前驱（`superseder_suppresses` 对 `is_deleted` 恒 false，MS10-T02 R6 精确保留既有语义）→ 无快照扫描产出已删除行的旧版本 T
- **证据**: MS15-Rest Iteration 002 Act Remaining Issues 2 跨进程探针 + Plan Review 独立复现（2026-09-12，归档 change `2026-09-12-ms15-rest-correctness-batch`）——`INSERT (1,10)` → 进程 A `UPDATE SET n=99 WHERE id=1`（affected 1）→ 进程 B `DELETE WHERE id=1`（affected 1）→ 进程 C 扫描 `[[1,10]]`、点查 `id=1` 空集（两步变更从扫描面消失）；对照 Z1 异键 update→delete、Z2 delete→update 均正常（`[[1,99]]`）；全裸名序列同样复现（与表名归一化无关，预存缺陷）；未被既有 867 测试覆盖
- **影响**: 语义为「既有行为未扩大」（R6 前同样重现）；MS10-T02 验收夹具 UPDATE/DELETE 域不相交故未触发；MS15-Rest 收尾探针实证该形态在真实跨进程 CLI 序列下两步变更丢失（原「未来混合负载」预判已现实化）
- **方案**: 抑制谓词区分「已提交墓碑」（应抑制整条链）与「未提交墓碑」（不抑制、回溯前驱）——需对照 WAL committed 集合或 header 编码扩展
- **状态**: promoted（MS09-T01 实施——DELETE 墓碑独立版本 slot 自描述删除者 + `superseder_suppresses` 按删除者提交状态（已提交抑制整链 / 未提交·回滚回溯前驱）+ 恢复两态一致，spec `mvcc-tombstone-visibility`，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`）

## I034: 裸 DataScan 子集投影的 CLI 表头返回全 schema

- **分类**: 正确性 / CLI 渲染
- **问题**: 无 WHERE 的裸 DataScan 子集投影，CLI 表头经 `get_plan_output_columns`（plan 节点 columns 元数据）返回全 schema 而行已按 projection 裁剪——`SELECT name FROM t` 表头 `["id","name"]`、行 `[["Alice"]]`；带 WHERE 的 IndexScan 路径表头正确（`["name"]`）。spec `cli-noninteractive-shell` R6 S1「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足
- **证据**: MS10-T04 Plan Review finding 5 独立探针复现（2026-09-09，revision `8827700`）；`tests/projection_test.rs`（MS10-T01）只断言 lib 行形状，未覆盖 CLI 表头
- **影响**: json 输出 `columns` 与 `rows` 字段数不一致，机器消费需二次裁剪；表格输出表头错位
- **方案**: `get_plan_output_columns` 对裸 DataScan 节点按 `projection` 索引裁剪表头；修正后同步校准 `tests/cli_test.rs::test_multi_statement_sequential_render` 的注释登记
- **状态**: promoted（2026-09-12 并入 MS15-Rest 实施——`get_plan_output_columns` 新增 `projected_columns` helper：DataScan 臂必需应用 projection、Scan/IndexScanAll 恒等加固、IndexScan 保持构造期收窄；change 归档 `openspec/changes/archive/2026-09-12-ms15-rest-correctness-batch/`，spec `cli-noninteractive-shell`「扫描执行器真投影」修改 + cli_test 表头用例）

## I035: no-FROM SELECT 不受支持

- **分类**: 功能 / SQL 能力边界
- **问题**: `SELECT 1`（无 FROM 子句）在 plan 阶段报 `Plan error: Missing required field: FROM clause`（exit 3）——MS10-T04 S5 分号边界见证原拟 SQL 因此不可达，修订为含 FROM 等价 SQL（用户批准，见归档 change Blocker Handoff/Resolution）；`tests/cli_test.rs:481-482` 既有注释同证
- **证据**: MS10-T04 Act Blocker Handoff + Plan Review 独立复现（2026-09-09，revision `5c42ec8`）
- **影响**: 常量表达式查询、`SELECT current_setting()` 类无表探测不可达；agent/脚本日常探测用法受挫
- **方案**: planner 增加 no-FROM SELECT 臂（单行虚拟输入），属引擎能力扩展，需独立 change
- **状态**: promoted（MS13-T03 已实施——`SingleRowExecutor` 虚拟单行 + 九项拒绝面点名，spec `no-from-select`，change 归档 `openspec/changes/archive/2026-09-23-ms13-analytics-functions/`；台账状态补转勘误 2026-09-24）

## I036: planner 简单 PK 等值 + 非 Int 字面量路由 Filter(Scan) 对无键行不可达

- **分类**: 正确性 / planner 路由
- **问题**: `WHERE <键列> = <非 Int 字面量>`（如 String 首列隐式 PK 表的 `WHERE s = 'x'`）经 `has_pk_equality` 结构化判定（不问可键控性）生成 Filter(Scan)，`ScanExecutor` 走 `index_manager.scan_all()` 索引遍历——键位不可键控（NULL/非 Int）的行落库不入索引（MS10-T05 001-rework 语义），经该 WHERE 形态不可达；无键行仅经非 PK 谓词或无 WHERE 的 DataScan 路径可见
- **证据**: MS10-T05 Iteration 001 001-rework Act Deviation 1 真二进制探针 + Plan Review 独立核实 `src/parser/planner/query.rs:430-475`（2026-09-09，归档 change `2026-09-09-2026-09-09-ms10-t05-lifecycle-subcommands`）；MS11-T03 双重实证同一根因的全空结果形态（2026-09-11，归档 change `2026-09-10-ms11-t03-scalar-functions`——无声明 PK 表首列字符串 Eq 经 `has_pk_equality`→Filter(Scan) 回退返回空行集 exit 0，pristine master 源码级探针 Eq→rows:[] vs Ne/Gt 正常 + CLI 探针，Plan Review 独立复现；测试以声明 PK 表形规避并注记）
- **影响**: 对含无键行的表按键位等值过滤漏行（静默不完整结果）；此前该形态表恒为空、行为不可观察
- **方案**: planner 对键位等值 + 不可键控字面量回退 DataScan 行内过滤（`to_key()==None` 时禁用索引路由）；属 planner 路由面，需独立 change
- **状态**: promoted（2026-09-12 并入 MS15-T01 实施——`has_pk_equality` 分支条件收窄 + 新增 `has_non_keyable_pk_literal_leg` 分类 helper，不可键控字面量腿落入既有 OR/下推臂；change 归档 `openspec/changes/archive/2026-09-12-ms15-t01-keyless-eq-routing/`，spec `planner-key-equality-routing`）

## I037: UPDATE 键位为无键值后运行期旧键索引条目指向无键版本

- **分类**: 正确性 / update 执行器索引维护
- **问题**: 键行经 `UPDATE SET <键列> = NULL`（或不可键控值）后，`UpdateExecutor` 以旧 key 无条件 `index_manager.update(&self.key, new_row_id)`（`src/executor/update.rs:130-133`）——旧键条目指向键位已为 NULL 的版本：运行期对该键值 INSERT 被 DuplicateKey 误拒（无行实际持有该键值）、点查可达性语义含混；崩溃恢复后重建自然清除（无键版本不入重建索引），运行期与恢复后两态不一致
- **证据**: MS10-T05 Iteration 001 001-rework Plan Review Finding 5 代码核实 + `tests/keyless_row_test.rs` 恢复面行为（2026-09-09，归档 change 同上）；修复前该形态 Update 重放 RedoFailed（库不可打开），001-rework 后恢复面正确
- **影响**: 运行期唯一性检查误拒；运行期/恢复后索引内容不一致（重开自愈）
- **方案**: update 执行器对新值 `to_key()==None` 改为删除旧键条目（`index_manager.delete`）而非 update；行为变化需回归 `tests/keyless_row_test.rs` 与恢复套件
- **状态**: promoted（2026-09-12 并入 MS15-Rest 实施——`UpdateExecutor` Step 7 分支化：新值不可键控改 `index_manager.delete(&self.key)`，运行期/恢复两态一致；change 归档 `openspec/changes/archive/2026-09-12-ms15-rest-correctness-batch/`，新 spec `update-index-maintenance`；邻接 rekey 形态登记 I047）

## I039: 多代 dump/restore 表名引号膨胀（ObjectName Display 即表名）

- **分类**: 正确性 / dump 保真
- **问题**: 引擎以 ObjectName 的 Display 形式为表名（`pipeline.rs:950/963`、`query.rs:101`、`ddl_dml.rs:287`）——带引号 DDL restore 重建后表名含引号字符，对该库再 dump 出现引号膨胀（`"items"` → `"""items"""`）；一代往返数据等价（`test_dump_restore_roundtrip_full_shape` 锁定），多代往返表名不保真；schema 命令对带引号建表同病
- **证据**: MS10-T05 Iteration 001 000-initial Act Deviation 1 探针 + Remaining Issues #2（2026-09-09，归档 change 同上）
- **影响**: 多代 dump→restore 链路表名逐代膨胀；Display 安全表名（CLI 常规路径）不受影响
- **方案**: 表名解析侧归一化（去引号）或 DDL 生成侧条件引号；涉引擎表名语义，需独立调查
- **状态**: promoted（2026-09-12 并入 MS15-Rest 实施——方案 A 解析侧归一化：`ast.rs` 新增 `object_name_to_table_name`（`Ident.value` 去引号 + lowercase + `.` 连接），11 处表名消费点统一经 helper，带引号与裸名拼写等价；change 归档 `openspec/changes/archive/2026-09-12-ms15-rest-correctness-batch/`，新 spec `table-name-resolution`；dump 侧 `select_all_rows` 随之经 `quote_ident` 包裹（转义名 catalog dump 可用 + 多代恒等，同 change Iter 002 001-rework T9-R1））

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
- **状态**: promoted（2026-09-23 并入 MS17-T02 缺陷清账——env 用例合并/串行化方案随 change 调查定稿；消除全量假失败源以稳定 MS17 全量验证门）

## I042: 边界子句拒绝 × 活跃会话事务组合路径无 e2e 锁定

- **分类**: 测试覆盖 / CLI 会话
- **问题**: spec `sql-transaction-statements` R2 各拒绝场景均为独立调用、空闲会话；「会话事务活跃中遇边界子句」（如 `BEGIN; INSERT ...; SAVEPOINT sp1`）的组合路径无 spec 场景与 e2e 用例——该路径行为由 `run_sql` Err 分支 + D5 收尾规则组合产生（拒绝不改变会话态、回滚属收尾），已由 Plan Review 二进制探针验证正确（exit 3 + D3 点名文案 + 事务上下文后缀 + 重开无残留）但无回归锁定
- **证据**: MS11-T02 Plan Review Finding F3 探针（2026-09-10，change `2026-09-10-ms11-t02-sql-transaction-statements`，iterations/001-cli-session/000-initial.md）
- **影响**: 未来重构 `run_sql` 错误路径时该组合语义可能静默回归；无当前正确性问题
- **方案**: `tests/tx_statement_test.rs` 增加组合场景 e2e（可选测试加固，随下一次触碰 CLI 会话面的 change 顺带实施即可）
- **状态**: planned（已排期 MS19-T03——2026-09-24 初版后第一批路线规划补充；可随 T01/T02 触碰 CLI 会话面时顺带）

## I043: 标量函数极端输入边界（abs i64::MIN 溢出 / round 极端 digits 非有限值）

- **分类**: 健壮性 / 标量函数
- **问题**: `abs(-9223372036854775808)`（i64::MIN）按契约实现为 `i64::abs()`（`src/executor/function.rs` ABS 臂），debug 构建溢出 panic、release 回绕为负；`round(x, digits)` 按 `10f64.powi(digits)` 乘除实现，|digits|>308 时 factor 溢出为 inf/0，结果 inf/NaN（SQLite `round(1,1000)=1.0`）。spec `sql-scalar-functions` R3 仅锁定常规值（`abs(-5)`/`round(123.4,-1)` 等），两边界均不在锁定面
- **证据**: MS11-T03 Iteration 001 Act Remaining Issues #1/#2（2026-09-11，归档 change `2026-09-10-ms11-t03-scalar-functions`；Plan Review 独立核实实现契约与公式）
- **影响**: 极端输入下 panic/非有限值；常规分析负载不可达，无当前正确性问题
- **方案**: abs 改 `checked_abs` 显式溢出报 `ValueError` 或文档化回绕语义；round 对 |digits| 设上限截断或文档化——随下次触碰 `function.rs` 的 change 顺带评估，需用户裁定方向
- **状态**: promoted（MS13-T02 已实施——abs 改 `checked_abs` 显式溢出错误、round ±308 SQLite 对齐饱和，spec `sql-scalar-functions` R3 修改，change 归档 `openspec/changes/archive/2026-09-23-ms13-analytics-functions/`；台账状态补转勘误 2026-09-24）

## I044: 标量函数名大小写不敏感缺 SQL 层测试见证

- **分类**: 测试覆盖 / SQL 函数层
- **问题**: spec `sql-scalar-functions` R1「函数名匹配 SHALL 大小写不敏感」无 SQL 层 e2e 见证——`tests/` 全部使用小写形态，无 `UPPER(...)`/`Abs(...)` 等大写/混合变体用例（tests/ 全目录 grep 实证）；实现层成立（`ast.rs` 两放行门与 planner 函数臂均先 `to_uppercase()` 规范化再查注册表，Plan Review 代码核实），但该 SHALL 子句在 23 场景集与两轮 RTM 中均无对应场景
- **证据**: MS11-T03 Plan Review Finding F1（2026-09-11，归档 change 同上；大写变体 grep + 两处规范化点核实）
- **影响**: 未来重构函数名匹配路径时大小写语义可能静默回归；无当前正确性问题
- **方案**: `tests/scalar_function_test.rs` 增加大写/混合大小写变体用例（可选测试加固，随下次触碰函数面的 change 顺带实施）
- **状态**: promoted（MS13-T02 已实施——`scalar_function_test` 增至 32 含大写/混合变体见证，change 归档 `openspec/changes/archive/2026-09-23-ms13-analytics-functions/`；台账状态补转勘误 2026-09-24）

## I046: 可键控 Int 字面量 + 非 Int 键列等值经 IndexScan 静默漏行（形态 2）

- **分类**: 正确性 / planner 路由
- **问题**: 键位等值腿字面量可键控但键列类型不容纳该键时，`extract_pk_from_where` 成功返回索引键 → `IndexScan` 点查空索引 → 静默漏行。实锤形态：Float 隐式 PK 表 `t(f FLOAT, n INT)` 行 `(5.0, 1)`，`WHERE f = 5`——Int 字面量 `to_key()==Some(5)`，行值 5.0 按 `Value::equals` Int↔Float 隐式转换本应匹配，实测 `rows:[]` exit 0（2026-09-12 探针）。String/Bool 键列 + Int 字面量因跨类型 equals 恒 false 而巧合正确，仅 Float（Int↔Float 隐式转换）真实漏行
- **证据**: MS15-T01 调查新发现（2026-09-12，归档 change `openspec/changes/archive/2026-09-12-ms15-t01-keyless-eq-routing/` proposal Out of Scope 形态 2 + design D4 残差 1；用户裁定采纳独立后续 change、方向 B 为候选）
- **影响**: 对含无键行的 Float 键列表按 Int 字面量等值过滤漏行（静默不完整结果）；MS15-T01 修复（按字面量可键控性判定）不覆盖此形态
- **方案**: 方向 B 键列类型感知路由——planner `register_table` 加性传递列类型（`pipeline.rs:996-1001` 调用点已有 `ColumnType` 可用），键列非 Int 时键位等值形态统一回退 DataScan；MS15-T01 的路由判定点即方向 B 将来落点，扩展不冲突
- **状态**: promoted（2026-09-13 并入 MS16 实施——`PlanBuilder` 加性传递键列声明类型（`primary_key_types`/`set_pk_column_type`）+ 两处判定门（`extract_pk_from_where` 门 1 + 非 PK 臂条件扩展门 2），键列声明非 Int 时键位等值形态统一回退 DataScan/Filter(DataScan)，行集按 `Value::equals` 行内求值；change 归档 `openspec/changes/archive/2026-09-12-ms16-correctness-batch/`，修改 spec `planner-key-equality-routing` 新增「键列类型感知路由」）

## I047: UPDATE 键列 SET 为另一可键控值（rekey）旧键条目残留且新键不可达

- **分类**: 正确性 / update 执行器索引维护
- **问题**: `UPDATE SET <键列> = <另一可键控 Int>`（如 5→7）时 `UpdateExecutor` 仅 `index_manager.update(&old_key, new_row_id)`——旧键条目残留指向新版本、新键 7 无索引条目：新键点查经索引不可达（静默空集）、旧键 INSERT 被 DuplicateKey 误拒（无行实际持有旧键值）、崩溃恢复重建索引后两态不一致（重建后旧键条目消失，以数据页为准）
- **证据**: MS15-Rest 调查新发现 + Iteration 002 T7 等价用例初稿同键 rekey 形态探针实证（2026-09-12，归档 change `2026-09-12-ms15-rest-correctness-batch` proposal 默认假设 2 + design D2 + Act Deviation 1——同键 rekey 后 `SELECT WHERE id=2` 空集；用户批准排除出该 change 范围）
- **影响**: rekey 后新键不可达、旧键误拒；与 I037（键位无键值清理，已实施）同面相邻——I037 修复只覆盖新值不可键控分支，rekey（新值可键控且 ≠ 旧键）行为不变
- **方案**: rekey 判定（新旧键均可键控且不等）改为删旧键条目 + 插新键条目；新键撞已有行 DuplicateKey 拒绝（与恢复侧重建重复 PK 显式报错一致）；需独立 change
- **状态**: promoted（2026-09-13 并入 MS16 实施——`UpdateExecutor` 前置块碰撞预检（新键 `search` 命中即 `DuplicateKey`，任何写入前零副作用）+ Step 7 三分支（NULL 删旧键〔I037 原样〕/ 同键 update〔原样〕/ rekey 先 `delete(old)` 后 `insert(new)`）；change 归档 `openspec/changes/archive/2026-09-12-ms16-correctness-batch/`，修改 spec `update-index-maintenance` 新增「键位 rekey 后索引条目一致」；6 个依赖缺陷行为的 M10 直连执行器测试按 BH-3 裁定校准）

## I048: import 实参插值对历史带引号表名不可达

- **分类**: 正确性边界 / lifecycle import
- **问题**: `import` 以 CLI 实参原文插值构造 `INSERT INTO {table}`（`src/cli/lifecycle.rs:467`）——表名解析归一化（I039 实施）后，catalog 名含引号字符的表（历史带引号 restore 产物）经 import 实参约定不可达：实参需与 catalog 名逐字比对（含引号），而带引号实参写入 SQL 经解析去引号后与 catalog 名不符（实参比对与解析文本双重转义矛盾）
- **证据**: MS15-Rest Plan Review Finding 7 代码核实（2026-09-12，归档 change 同上）；spec `table-name-resolution` R2 仅覆盖 dump/schema，import 无 requirement 面；归一化前该形态经 Display 凑巧可达
- **影响**: 极窄角落（历史带引号表名 × import）；design D3「历史带引号表名可达性收缩」预发布边界内，新库无影响
- **方案**: import 表名实参经 `quote_ident` 包裹或文档化该边界；随下次触碰 lifecycle/import 面的 change 顺带
- **状态**: promoted（2026-09-23 并入 MS17-T02——原 MS14 随带项随 MS14 裁剪并入；quote_ident 包裹或文档化，可裁，随 change 调查定稿）

## I059: 跨数据库文件交互（ATTACH 式多文件关联检索与物化）

- **分类**: 功能 / 多库交互
- **问题**: `Database` 单文件绑定（单 table_manager/FileStorage/WAL/事务管理器，`database.rs:42-121`），CLI 无跨 .db 文件关联检索与物化能力——多库场景（跨库查询、抽取、合并、分析）需人工 dump/restore 中转
- **用户方向**（2026-09-24）: 两个或多个 db 文件经命令行关联检索；跨库增删查改；跨库取视图物化为新库（「数据库视作表」）
- **方案**: SQLite ATTACH 同型（嵌入式正统设计；PostgreSQL 单连接锁死单库为反例）——`Database` 增 attach 注册表 `Map<别名, AttachedFile>`（各持 table_manager/storage/BufferPool）+ 表名命名空间「别名.表」解析（I039 落地的 11 处表名消费点扩展）+ 每文件独立 RC 快照；文件锁为 try_lock 不等待，交叉 attach 死锁结构性不可能（一方直接 `DatabaseLocked` exit 4）。两期拆分：一期只读跨查 + CTAS（`CREATE TABLE AS SELECT`，单独即单库通用 SQL 增益；`new` + attach + CTAS 组合覆盖物化全场景）；二期跨文件 DML（每文件独立提交、非原子 v1 语义文档化——SQLite 跨 attach 原子提交亦需 master journal 级机制）
- **身份过滤器判定**: 异步/嵌入式/CLI 三词全沾（attach 文件走同一异步扫描路径 / 无 daemon 多文件互查的正统设计 / 一行命令跨库抽取合并分析、agent 场景强化）——判定依据见 R25 分析「定位裁定与决策过滤器」节
- **状态**: planned（已排期 MS20 一期 T01/二期 T02——2026-09-24 初版后第一批路线规划；与 crate 化路线 B 顺风——attach 注册表催生「多文件会话」抽象）

## I061: 删库子命令（`rtsql delete <db>` 管理命令行）

- **分类**: 功能 / CLI 生命周期
- **问题**: 无删库子命令——生命周期子命令只有 `new/list/schema/dump/restore/import`（`rtsql --help` 实证），SQL 层 `DROP TABLE` 只删表、库文件与伴生文件仍在；删库只能文件级 `rm <name>.db <name>.wal <name>.checkpoint` 三件套
- **用户方向**（2026-09-24）: 卸载脚本 `install.sh --uninstall --purge-data` 虽有数据清理能力，但那是卸载场景；删库作为日常管理命令行也应存在
- **证据**: 2026-09-24 用户手动部署验证时发现（本会话 `rtsql --help` 子命令清单确认无 delete/drop 类命令；improvements 台账与 tasks 路线均无此条目，无重复登记）
- **影响**: 手工 rm 需要用户了解伴生文件布局（`.db`/`.wal`/`.checkpoint`），漏删伴生文件会残留脏现场；agent/脚本管理场景缺一行式命令，`list` 可枚举但不可回收
- **方案**: 新增 `rtsql delete <db>`（或 `drop-database`）子命令——复用 `resolve_existing_db` 定位（裸名集中存储区 / 含 `/` 路径），删除主文件与 `.wal`/`.checkpoint` 伴生文件并报告释放结果；打开中（advisory 文件锁占用）SHALL 显式拒绝；加密库为文件级操作无需密钥；命令命名、dry-run/确认交互、路径形态边界与 `install.sh --purge-data` 语义关系随 change 调查定稿
- **状态**: planned（已排期 MS19-T01——2026-09-24 初版后第一批路线规划）

## I062: close()/checkpoint 在无未决 WAL 记录时跳过全量 checkpoint

- **分类**: 性能 / CLI one-shot 路径
- **问题**: `Database::close()` 无条件转调全量 checkpoint（`src/database.rs:250-251` → `checkpoint_manager.checkpoint`：刷脏页 + 写位点 + WAL 重写截断）——零写入会话（WAL 无未决记录）也全价执行；one-shot `SELECT 1` 实测 10.8ms/次（sqlite3 同负载 1.15ms），固定开销主要来自 runtime 构建与 close checkpoint
- **证据**: 2026-09-24 SQLite 对比测量（README 对比板块、R26 runbook：50 次时延循环）；代码面 `src/database.rs:250-251`
- **影响**: agent/脚本高频 one-shot 场景每次多付毫秒级固定开销；无正确性影响
- **方案**: checkpoint 前检查 WAL 未决记录（WalWriter 已有文件长度查询，writer.rs:180-188）为零/低于阈值时跳过重写直接返回；「close 即落盘」承诺语义不变——无未决记录时本无落盘工作；注意与 I064 文档口径一致
- **状态**: planned（已排期 MS18-T01——2026-09-24 初版后第一批路线规划；小改动高收益）

## I063: CLI tokio runtime 规格评估（multi_thread 默认 → 按负载选 current_thread）

- **分类**: 性能 / CLI 资源占用
- **问题**: `src/main.rs:3` `#[tokio::main]` 默认 multi_thread + num_cpus worker（实测机 32 线程）——one-shot CLI 无并发连接需求，实测峰值 RSS 16.7MiB（sqlite3 同负载 4.1MiB），worker 线程与运行时结构为大头（BufferPool 本身仅 100 页 ≈400KB）
- **证据**: 2026-09-24 SQLite 对比测量（README 对比板块、R26 runbook）；代码面 `src/main.rs:3`
- **影响**: CLI 内存占用 ~4x 与部分启动时延；无正确性影响；Server 面独立 runtime 不受影响
- **方案**: CLI 面实测 `#[tokio::main(flavor = "current_thread")]`（或 Builder 定制）——需验证引擎内部 `spawn_blocking`（WAL/页 I/O）与 DataScan 预取在 current_thread 下的行为与吞吐；以 RSS/时延/吞吐三组数据决定是否切换（MS08「先量化再决定」纪律适用）
- **状态**: planned（已排期 MS18-T02——2026-09-24 初版后第一批路线规划；先量化再决定）

## I068: ALTER TABLE 与二级索引（CREATE/DROP INDEX）

- **分类**: 功能 / SQL DDL 面
- **问题**: 语句面无 `ALTER TABLE ADD/DROP COLUMN` 与 `CREATE INDEX`/`DROP INDEX`（索引仅 PK 自动建）；同面缺口还有 `CREATE VIEW`/`TRUNCATE`/`EXPLAIN`（R18 主题 2 语句清单）
- **证据**: R18 主题 2（`src/parser/planner/mod.rs:90-93` 6 种语句臂，其余 `UnsupportedStatement`）+ 主题 3 差距清单「ALTER TABLE ADD/DROP COLUMN、CREATE INDEX | 中-大：触及 catalog/序列化/重建路径」
- **影响**: 列结构演进只能 dump→改 DDL→restore 重建；非 PK 查询列无二级索引可用
- **方案**: ALTER 与 CREATE INDEX 各触及 catalog/序列化/重建路径，评估中-大，需独立 change（VIEW/TRUNCATE/EXPLAIN 届时一并裁定是否并入范围）
- **状态**: planned（已排期 MS21-T01/T02——2026-09-24 初版后第一批路线规划）

## I070: 错误信息可操作化（特性名携带与错误分类）

- **分类**: 可用性 / 错误报告
- **问题**: `UnsupportedStatement`/`UnsupportedExpression` 不携带特性名（报错无法直接得知不支持什么）；PG 协议错误码统一 58000；CLI 退出码分类（0/1/2/3/4/5）与各 change 点名文案已零散落地，系统性的 PlanError 特性名与错误分类未做
- **证据**: R18 主题 3 差距清单「错误信息可操作化 | UnsupportedStatement/Expression 无上下文；PG 统一 58000 | 小：PlanError 携带特性名；错误码分类」
- **影响**: 排错需读源码或试错；协议面错误不可按类消费
- **方案**: `PlanError` 携带特性名 + 错误分类（评估小改动面），可独立小 change 或随下次触碰错误面的 change 顺带
- **状态**: planned（已排期 MS19-T02——2026-09-24 初版后第一批路线规划）

## I072: 文件级备份/移动配对语义验证与文档化

- **分类**: 文档 / 数据安全边界
- **问题**: 文件对（`.db` + `.wal`）是移动/备份的最小单位——只拷主文件在 checkpoint 后等价、否则丢尾部事务；`with_extension("wal")` 的扩展名替换语义下移动/重命名/拷贝的配对行为未实测，用户文档也未声明「备份首选 dump/restore 逻辑导出」
- **证据**: R18 主题 5 推论（文件对最小单位 + 逻辑导出首选）+ 未确认项 5（`.wal` 与主文件移动/重命名配对行为未实测）
- **影响**: 用户以文件拷贝做备份可能静默丢尾部事务且无文档提示
- **方案**: 实测移动/拷贝/重命名配对行为矩阵 + README 文档化备份边界（dump/restore 首选）；验证+文档小项，与 I061（删库子命令）同面可顺带
- **状态**: planned（随 MS19-T01 顺带实施——2026-09-24 初版后第一批路线规划补充）
