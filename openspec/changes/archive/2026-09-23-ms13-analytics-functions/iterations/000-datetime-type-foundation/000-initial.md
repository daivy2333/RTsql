# Iteration 000 / Cycle 000: datetime 类型底座

## Plan Context

- Status: ready
- Iteration: 000-datetime-type-foundation
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: DATE/TIMESTAMP 全链可用（建表落列/字面量写入/比较排序/PK 路由/CAST/四格式渲染/dump-restore/CSV/恢复两态一致），后续 Iteration 在其上叠加函数与分桶
- Verification boundary: `tests/datetime_type_test.rs` 全绿 + 既有全量测试零修改通过
- Diagnostic boundary: `src/executor/{value,value_ref,datetime}.rs`、`src/storage/page_format/tuple.rs`、`src/storage/catalog.rs`、`src/parser/planner/{ddl_dml,expression}.rs`、`src/parser/ast.rs`、`src/executor/{insert,update,sort,predicate}.rs`、`src/cli/lifecycle.rs`、`src/pipeline.rs`
- Deferred tasks: T6–T13（Iteration 001/002）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change proposal 决策 1–2、DA1–DA6、design D1–D9/D16
- Excluded scope: 日期函数族/INTERVAL/GROUP BY 扩展/no-FROM/CLI 分析命令/I043/I044（Iteration 001/002）

**Objective**

`DATE`/`TIMESTAMP` 作为一等值类型从 DDL 到渲染全链贯通：类型化与裸字符串字面量可写、时间序可比可排、非 Int PK 路由正确回退、CAST 双向可达、dump/restore/CSV 往返恒等、崩溃恢复后等值回读；既有五类型行为零回归。

**Background**

tasks.md MS13-T01（分析最高频维度的类型系统扩展，R18 主题 7）。当前 `convert_data_type` 对 DATE/TIMESTAMP 回退 String（`ddl_dml.rs:306` `_ => ColumnType::String`），无真日期类型。用户决策（2026-09-23）：双类型 + 写入强制解析 + 比较严格。探针实证（revision `7364bc9` 二进制）：`CREATE TABLE ev (d DATE)` 建表成功但 schema 显示 `d STRING`。

**Investigation Facts**

- Current Baseline: revision `7364bc9`，936 tests / 0 failed / 2 ignored（SNAPSHOT 2026-09-14 记录，覆盖范围未变化）；工作区仅 docs 增量（MS09 收尾 sync），无代码改动。
- Current-State Evidence（全部 2026-09-23 实读）:
  - 类型链：sqlparser DataType → `executor::ColumnType`（`ddl_dml.rs:276 convert_data_type`，未知→String 兜底）→ `storage::page_format::ColumnType`（`plan.rs:198 to_schema_column`，executor String→storage String(255)）→ catalog `COL_TAG_*`（`catalog.rs:40-44`，现值 0x01–0x04；序列化 `catalog.rs:656-736`）。
  - `Value` 五变体（`value.rs:46`）；`ValueRef` 五变体 Copy（`value_ref.rs:14`）；`to_key()` 仅 Int（`value.rs:82`）。
  - tuple tag 0x01–0x05（`tuple.rs:16-20`）；`compute_tuple_size`/`serialize_tuple`/`deserialize_tuple`/`deserialize_value_refs` 四函数按 tag 分派；损坏/截断 → StorageError::Io(InvalidData/UnexpectedEof)。
  - WAL 记录 `tuple_data: Vec<u8>` 不透明（`wal/record.rs:53`）；恢复键提取 `recovery.rs:203` `pk_value.to_key()` None → keyless 桶（`recovery.rs:39/251`）——非键控值自动两态一致。
  - 比较语义：`equals` 跨族返回 false（`value.rs:118`）；`gt/lt/ge/le` 跨族 `Err(TypeMismatch)`（`value.rs:140` 等）；ComparisonPredicate NULL→Unknown（`predicate.rs:103`）。
  - 排序 `sort.rs:103 compare_values`：显式同型臂 + `_ => Ordering::Equal` 兜底（新类型不补臂会被兜底吞掉排序）。
  - MIN/MAX 经 `lt_agg`（`value.rs:232`，跨族 false 兜底）；SUM/AVG 经 `add/div`（`value.rs:220/244`，`_ => Value::Null` 兜底）。
  - 字面量入口四点：`build_expression`（`expression.rs:170`）、`build_where`（`expression.rs:489`）、`extract_insert_values`（`ddl_dml.rs:219`，仅 `Expr::Value`）、UPDATE SET（`ddl_dml.rs:515`，仅 `Expr::Value`/NULL）。`value_from_sqlparser`（`parser/value.rs:8`）只认 Number/SingleQuotedString/Null/Boolean。
  - `ast.rs extract_columns`（`ast.rs:34`）放行清单：Identifier/CompoundIdentifier/Function(注册名/COALESCE)/Value(`_v` 命名)/Case/Cast/InList/Between/Like/IsNull/Trim/Ceil/Floor；**BinaryOp 不在清单**（探针：`SELECT 1 + 1` 含 FROM 形态报 `Unsupported statement type` exit 3）。
  - TypedString/Interval 在 sqlparser 0.44 可用（`Expr::TypedString { data_type, value }`、`Expr::Interval(Interval)`；registry 源码确认）。
  - InsertExecutor 持 `schema: Vec<ColumnType>`（`insert.rs:106`），MS16 键列类型预检在 `insert.rs:128`；UpdateNode 携带 column + new_value 常量（`ddl_dml.rs:535`）。
  - CAST：`CastType` Int/Float/String/Bool 四族矩阵（`predicate.rs:555`），跨族拒绝；planner CAST 目标映射在 `expression.rs:295`。
  - 渲染：`value_to_json`（`pipeline.rs:787`）是唯一 Value→json 点；CLI render 消费 serde_json（`render.rs:53`）；dump `sql_literal(v: &serde_json::Value)`（`lifecycle.rs:251`）+ dump 循环内 catalog 列可用（`lifecycle.rs:162`）；`create_table_sql` 类型渲染 `lifecycle.rs:567`；`csv_value(field, col_type)`（`lifecycle.rs:502`）空字段有类型默认。
  - MS16 键路由：`pk_type_known_non_int` 两判定门（`query.rs extract_pk_from_where`/`has_pk_eq` 分支）；`has_non_keyable_pk_literal_leg` 只认 `Expr::Value` 腿——TypedString 腿不满足 `has_pk_equality` 结构探测，落入 OR/下推臂（双保险，无需改动）。
- Code and Critical Path: DDL（ddl_dml→plan.rs→catalog）→ 写入（insert/update 序列化前）→ 读取（tuple 反序列化×2→value_to_json/render）→ 比较（predicate/sort/aggregate）→ 导入导出（lifecycle）。变更均为类型分派臂的加性扩展，无控制流重写。

**Implementation Guidance**

- T1 先行独立 RED（纯函数模块，无下游依赖，可最快建立测试见证）；T2 依赖 T1 的解析/格式化；T3–T5 按链路顺序。
- `datetime.rs` civil 算法用 Howard Hinnant `days_from_civil`/`civil_from_days`（公有领域，i64 内部运算）；对外 API 以 i32 天数/i64 微秒为主。
- 单测锚点值：闰年序列 2024-02-29 合法/2023-02-29 非法/1900 非闰/2000 闰；1970-01-01 的 Hinnant `days_from_civil` 值为 719468（基准 0000-03-01），而 `Value::Date` 基准为 0001-01-01（两基准差 306 天，即 Date(1970-01-01)=719162）——常量以 T1 单测内函数互证推导，不手抄（避免基准混淆）。
- 序列化臂模式照抄 TAG_FLOAT（定长负载）写法；catalog 臂照抄 COL_TAG_BOOL。
- InsertExecutor 收口检查放 MS16 键位预检之前（逐列循环先于任何索引访问，保证零副作用拒绝）。

**Behavioral Change**

- 当前：DATE/TIMESTAMP DDL 回退 String；无日期字面量；无日期比较/排序。
- 目标：真类型全链（见 Objective）；行为变化点三个——`CREATE TABLE ... DATE/TIMESTAMP` 落真类型（既有库不受影响，仅新 DDL）；`ast.rs` BinaryOp/TypedString 放行（WITH-FORM 算术 SELECT 从错误变可达，no-from-select spec 场景已锁定，本 Iteration 只解锁放行面、SingleRow 属 Iteration 002）；日期列写入强制解析错误面新增。
- 接口语义：新 `StorageError::InvalidDateTime { value: String, expected: &'static str }`（Display 含两字段）；`PlanError::ParseError` 复用（字面量解析失败点名「invalid DATE/TIMESTAMP literal」+ 原值）。

**Task Contracts**

### T1: datetime 日历数学模块（RED 先行）

- Requirement/Scenario: datetime-type-system R1（序列化往返的解析/格式化底座）/ R3（解析格式）；设计 D1/D4
- Depends on: None
- Targets: `src/executor/datetime.rs`（新文件）
- Current behavior: 不存在日期解析/格式化/日历算术模块
- Required behavior: 提供 `parse_date(&str) -> Option<i32>`（YYYY-MM-DD）、`format_date(i32) -> String`、`parse_timestamp(&str) -> Option<i64>`（`YYYY-MM-DD[ T]HH:MM:SS[.f{1..6}]`，纯日期→零点）、`format_timestamp(i64) -> String`（微秒≠0 追 6 位小数）、字段抽取 `date_fields(i32) -> (i32 y, u32 m, u32 d)`、`ts_fields(i64)`、截断 `trunc_ts(i64, unit) -> i64`（year/month/day/hour/minute/second 清零语义）、日历范围校验（0001-01-01..9999-12-31，越界 None/错误）、civil 互转函数
- Required changes: 新模块 + `executor/mod.rs` 导出；单元测试覆盖：闰年 400 年规则（1900 非/2000 闰/2024 闰）、月末（01-31/02-28/02-29/04-30）、负值天数字段抽取、往返恒等（parse∘format = id）、边界年 0001-01-01 与 9999-12-31、微秒 6 位与 7 位拒绝、`T` 分隔符与空格、时区后缀拒绝、f>6 拒绝
- Preserve: 纯函数无状态无副作用；不引入 chrono/time 依赖
- Forbidden: 不得改动既有模块；不得引入 panic 路径（全部 Option/Result）
- Test witness: 模块内 `#[cfg(test)]`（先写后实现观察 RED：模块不存在 → 编译失败即 RED 起点，或先建空骨架断言失败）；命令 `cargo test datetime`
- GREEN condition: 全部新单测通过
- Verification: `cargo test datetime` exit 0
- Stop when: 算法实现与测试对闰年/月末语义冲突且无法以公有领域算法裁决时（返回 Plan）

### T2: Value/ValueRef/tuple/catalog 类型底座

- Requirement/Scenario: datetime-type-system R1（序列化往返/截断损坏拒绝）；设计 D1/D2/D7
- Depends on: T1（format/parse 用于 Display 与后续；本任务 Display 直接调 T1）
- Targets: `src/executor/value.rs`、`src/executor/value_ref.rs`、`src/storage/page_format/tuple.rs`、`src/storage/catalog.rs`、`src/executor/sort.rs`
- Current behavior: 五变体；TAG 至 0x05；COL_TAG 至 0x04；`compare_values` 无日期臂
- Required behavior: `Value::Date(i32)`/`Value::Timestamp(i64)` 全套方法臂——equals 同型/跨族 false、gt/lt/ge/le 同型 Ok/跨族 Err(TypeMismatch)、Hash 两臂、Display（T1 格式化）、to_key→None、as_value_ref/to_value、lt_agg 同型臂；`ValueRef` 两 Copy 变体同套；`TAG_DATE=0x06`（4B LE）/`TAG_TIMESTAMP=0x07`（8B LE）四函数臂；双 `ColumnType` 加变体；catalog `COL_TAG_DATE=0x05`/`COL_TAG_TIMESTAMP=0x06` 序列化/反序列化臂；`compare_values` Date×Date/Timestamp×Timestamp 显式臂
- Required changes: 上述全部；`add/div` 无需改动（`_ => Null` 兜底即目标语义，验证不误加）；tuple 单测补 Date/Timestamp 往返/截断/未知 tag 0x08 拒绝/零拷贝往返；catalog 单测补两 COL_TAG 往返
- Preserve: 既有五类型全部路径逐字节不变（含 Float NaN Display、String 引号 Display）；反序列化按 tag 而非 schema 分派的既有语义
- Forbidden: 不改 WAL/恢复层（不透明流过是设计结论，勿加处理）；不改 to_key 键控面
- Test witness: tuple.rs/catalog.rs/value.rs 单测扩展（RED：新变体缺失时编译错即为见证起点；行为断言先于实现写入测试文件）
- GREEN condition: 新单测全绿 + 既有单测零修改
- Verification: `cargo test --lib` exit 0
- Stop when: 双 ColumnType 枚举出现无法加性扩展的穷尽匹配冲突（返回 Plan）

### T3: DDL 映射与 schema/dump 类型面

- Requirement/Scenario: datetime-type-system R2（建表落列/INTERVAL 拒绝）；设计 D3
- Depends on: T2
- Targets: `src/parser/planner/ddl_dml.rs::convert_data_type`、`src/executor/plan.rs::to_schema_column`、`src/cli/lifecycle.rs::create_table_sql`
- Current behavior: DATE/DATETIME/TIMESTAMP → String 兜底；INTERVAL → String 兜底（可建表！）；渲染 INT/FLOAT/BOOL/STRING 四臂
- Required behavior: `DataType::Date → ColumnType::Date`；`DataType::Datetime(_) | DataType::Timestamp(_, TimezoneInfo::None) → ColumnType::Timestamp`；`TimestampTz`/其他 Tz 变体、`DataType::Time*`、`DataType::Interval` → `PlanError::ParseError` 点名拒绝（含类型名）；未知类型回退 String 兜底保持；`to_schema_column` 两臂（无参直映射）；`create_table_sql` 渲染 `DATE`/`TIMESTAMP`
- Required changes: 上述三处 + convert_data_type 返回类型改为 Result（或内部映射+外部校验，实现选加性最小面；当前签名 `&self -> ColumnType`，改为 Result 时同步唯二调用点）
- Preserve: INT 族/VARCHAR 族/FLOAT 族/BOOL 族映射不变；错误文案风格与既有 ParseError 一致
- Forbidden: 不动约束提取/PK 提取路径
- Test witness: `tests/datetime_type_test.rs` 起步组——RED：`CREATE TABLE ev (d DATE, ts TIMESTAMP)` 后 schema 输出断言含 `DATE`/`TIMESTAMP`（当前输出 `STRING`，断言失败）；`CREATE TABLE t (i INTERVAL)` 期望报错（当前成功，RED）
- GREEN condition: 建表/schema/拒绝三用例转绿
- Verification: `cargo test --test datetime_type_test` exit 0
- Stop when: sqlparser DataType 变体形态与设计映射出现语义分歧（返回 Plan）

### T4: 类型字面量 + 写入强制解析

- Requirement/Scenario: datetime-type-system R3（类型字面量写入回读/裸字符串强制/非法拒绝/恢复两态）；设计 D4/D5/D6
- Depends on: T1, T2, T3
- Targets: `src/parser/planner/expression.rs`（build_expression/build_where 两 TypedString 臂）、`src/parser/planner/ddl_dml.rs`（extract_insert_values 臂 + UPDATE SET 臂）、`src/parser/ast.rs`（extract_columns 放行 TypedString + BinaryOp）、`src/storage/error.rs`（InvalidDateTime）、`src/executor/insert.rs`、`src/executor/update.rs`
- Current behavior: TypedString 在四处入口落入不支持兜底；INSERT/UPDATE 无类型感知；`SELECT 1+1`（含 FROM）Unsupported statement type
- Required behavior: 四入口 TypedString → `ConstantExpression(Value::Date/Timestamp)`（T1 解析；`data_type` 非 Date/Datetime/Timestamp → 既有 Unsupported 兜底）；InvalidDateTime 变体；InsertExecutor 序列化前逐列：目标列 Date/Timestamp 时值 ∈ {同族, Null, String(强制解析)} 否则 InvalidDateTime（含解析失败）；UpdateExecutor SET 同规则；ast.rs 放行 TypedString（`expr.to_string()`）与 BinaryOp（`expr.to_string()`）
- Required changes: 上述；WITH-FORM `SELECT id + 1 FROM t` 随放行解锁（has_expression_items→Projection 既有通路，无需新代码——验证用例即可）
- Preserve: `Expr::Value` 字面量路径零变化；MS16 键位预检顺序（收口检查不得晚于键位预检之后产生副作用——置于最前）；UPDATE 单 assignment 约束
- Forbidden: 不做比较侧自动转换（`WHERE d = '2024-01-15'` 保持跨族错误——比较严格是决策 2）；不做 INSERT 的 DEFAULT 语法扩展
- Test witness: datetime_type_test 写入组 RED——类型字面量 INSERT+SELECT 回读（当前 Unsupported statement type 失败）；裸字符串强制（当前落 String 值、断言 Date 失败）；`'not-a-date'`/`'2023-02-29'` 期望错误（当前成功落库）；close→reopen 等值；UPDATE SET 三形态；`SELECT id + 1 FROM t` 输出 2/3
- GREEN condition: 写入组全绿
- Verification: `cargo test --test datetime_type_test` exit 0
- Stop when: TypedString 在 INSERT/UPDATE 的 AST 形态与 sqlparser 实测不符（返回 Plan）

### T5: 比较/排序/PK 路由/CAST/渲染/导入导出收口

- Requirement/Scenario: datetime-type-system R4（时间序过滤/跨类型拒绝/PK 回退）、R5（CAST 双向/非法跨族）、R6（dump 恒等/CSV 落类型）、R7（零回归）；设计 D7/D8/D9
- Depends on: T4
- Targets: `src/executor/predicate.rs`（CastType + 矩阵 + evaluate_ref Copy 直回）、`src/parser/planner/expression.rs`（CAST 目标映射臂）、`src/pipeline.rs::value_to_json`、`src/cli/lifecycle.rs`（sql_literal 类型化 + csv_value 两臂）、测试文件收口
- Current behavior: CAST 目标四族；value_to_json 五臂；sql_literal 无类型感知；csv_value 四臂
- Required behavior: CastType::Date/Timestamp + 矩阵（String→解析/Date↔Timestamp 截断扩展与零点/日期→String 格式化/数值 Bool 跨族拒绝）+ evaluate_ref Date/Timestamp 直回 ValueRef；CAST 映射臂；value_to_json 两臂（String 格式化）；dump 对 Date/Timestamp 列输出 `DATE '...'`/`TIMESTAMP '...'`（列类型经 dump 循环 catalog 列）；csv_value 两臂（空→Null/非空 String 透传）；datetime_type_test 全套成型（~25：比较排序/跨类型错误/PK Date 路由键位等值可达/CAST 六形态/dump→restore→dump 恒等含日期列/CSV import 落类型与非法拒绝/json 字符串形态断言）
- Required changes: 上述；既有全量回归跑通（零修改）
- Preserve: 既有 CAST 全矩阵不变；sql_literal 其余形状不变；csv_value 既有四臂不变；json 特殊 Float→Null 语义不变
- Forbidden: 不动 sort.rs（T2 已完成 compare_values）；不动 MS16 路由代码（只验证）
- Test witness: datetime_type_test 收口组 RED——CAST('2024-01-15' AS DATE)（当前 Unsupported DataType 失败）；dump 恒等（当前日期不存在，先以 T4 后的 String 回退形态断言失败）；PK Date `WHERE d = DATE '...'` 可达（T4 后应已通，作回归锚点）；CSV import（当前 Date 列空字段走 String 默认）
- GREEN condition: datetime_type_test 全绿 + `cargo test --no-fail-fast` 全量既有 936 零修改通过 + clippy/fmt 0
- Verification: 全量命令 + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check`，exit 全 0
- Stop when: 全量出现与本 Iteration 变更因果的既有测试失败且三次修复无效（Gate 6）

**Invariants**

- 既有五类型全链行为逐字节不变（全量零修改是硬验收）。
- WAL/恢复/索引层零改动（不透明流过 + to_key None 是设计结论）。
- 严格类型比较哲学不变（唯一新增隐式转换 = 写入边界 String→日期，决策 2 授权）。
- 新增错误均为显式具名（InvalidDateTime / ParseError 点名），无静默降级。
- Evidence 预算与身份型证据禁令（CLAUDE 行为约束）。

**Non-goals**

日期函数族/INTERVAL/GROUP BY 扩展/no-FORM/CLI 分析命令/I043/I044；strftime/to_date；时区；日期键控；SUM/AVG 日期聚合。

**Acceptance**

datetime-type-system 全部 7 Requirement 场景经 `tests/datetime_type_test.rs`（~25 用例）+ tuple/catalog/value 单测 + 全量零修改覆盖（映射见 change tasks.md RTM 前 7 行）；T4/T5 各含恢复两态与 dump 恒等场景。

**Verification**

- `cargo test --test datetime_type_test`：全绿。
- `cargo test --no-fail-fast`：936 既有 + 新增全绿、零修改。
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`：0。
- 探针抽检：`rtsql ev "SELECT * FROM ev"` json 输出日期字符串形态（Act Response 记录）。

**Gate 2 Readiness**

| 检查项 | 状态 | 证据 |
|---|---|---|
| 无 Missing requirement | PASS | change tasks.md RTM 22 行全 Covered（7 spec 域 → T1–T13 全映射） |
| 无未批准 Simplified | PASS | 无 Simplification 列（需求零裁剪；DA1–DA11 经 Gate 1 批准） |
| 调查完整 | PASS | Investigation Facts 全部符号/行号/探针实证（2026-09-23 实读 revision 7364bc9；`SELECT 1+1` 与 DATE DDL 二进制探针） |
| 设计闭合 | PASS | design D1–D16；无影响契约语义的 TBD（R1–R4 均非阻塞且已定处置方向） |
| 任务可执行 | PASS | T1–T5 契约各含 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop |
| 分轮合理 | PASS | 3 Iteration 依赖有序；000/001/002 各含平衡审计段落 |
| 追踪完整 | PASS | RTM requirement→scenario→design→task→iteration→code→witness 链路齐 |
| 验证充分 | PASS | 每任务 RED→GREEN 见证 + Iteration 级全量零修改硬 Gate（T5/T13） |
| 无身份型证据工程 | PASS | Persisted Evidence none；验证全部为目标行为观察（CLAUDE 行为约束复核） |
| 无需 Act 决定的实质未知项 | PASS | R3（convert_data_type 签名改 Result 调用点）为机械同步；datetime 常量基准已消歧 |
| 产物一致 | PASS | proposal/design/tasks/specs/Cycle 互检；`openspec validate` PASS |
| Persisted Evidence 契约 | PASS | Mode none（白名单四问全否） |
| 用户批准计划 | PASS | 用户批准「批准，交付 Act」原话记录于会话（2026-09-23）；Gate 1 亦同日批准 |

**Persisted Evidence**

- Mode: none
- 全部验证经 Act Response 承载（命令/决定性输出/退出码）；无白名单情形（可低成本重跑、无一次性环境、无 Issue 现场）。
- Budget: 不适用（none）。

**Risks and Notes**

- R1 手写日历数学正确性——T1 穷举单测 + 已知锚点值；非阻塞。
- R2 BinaryOp 放行波及面——表达式项既有拒绝面（JOIN 混用/子查询混用/通配混用/聚合混用）对算术项同步生效，`SELECT id+1 FROM t` 解锁已入 spec；非阻塞。
- R3 `convert_data_type` 签名改 Result 的调用点数量——调查确认唯二（ddl_dml.rs:442 + expression.rs 注释提及的 CAST 侧独立）；若 Act 发现更多调用点按机械同步处理（非实质）。
- datetime.rs 常量（epoch 天数 719468）若与测试推演冲突，以 T1 单测互证修正——非契约语义问题。

## Act Response

- Status: reported

**Implemented**

T1–T5 全部完成（RED→GREEN 逐任务见证）：T1 `src/executor/datetime.rs` 日历数学模块（Hinnant civil 算法、DA5 定宽解析/格式化、`trunc_ts`、`coerce_datetime_write` 写入边界强制解析，31 单测）；T2 类型底座（`Value`/`ValueRef` Date/Timestamp 变体全方法臂、tuple TAG 0x06/0x07 四函数、双 `ColumnType` 枚举、catalog COL_TAG 0x05/0x06、`compare_values` 显式臂、`to_key→None`、`lt_agg` 同型臂、add/div Null 兜底验证）；T3 DDL 映射（`convert_data_type` 改 Result——Date/Datetime/Timestamp(None) 显式映射、Tz/Time/Interval 点名拒绝、String 兜底保持）；T4 类型字面量与写入强制（TypedString 四入口 plan 期解析、`StorageError::InvalidDateTime`、InsertExecutor 逐列收口先于 MS16 键位预检、UpdateExecutor SET 同规则、ast.rs TypedString/BinaryOp 放行、`BinaryArithExpression` 算术节点）；T5 收口（CAST Date/Timestamp 矩阵 + 目标映射、谓词层日期族比较严格性守卫、value_to_json/PG 协议 DA5 渲染、dump 类型化字面量 `typed_datetime_literal`、csv_value 透传、`column_type_sql` DATE/TIMESTAMP）。

**Changed Files and Symbols**

- 新增：`src/executor/datetime.rs`（parse_date/format_date/parse_timestamp/format_timestamp/date_fields/ts_fields/trunc_ts/TruncUnit/ts_to_date_serial/date_serial_to_ts/coerce_datetime_write + DAYS_0001_TO_1970 派生常量）、`tests/datetime_type_test.rs`（16 e2e）
- 修改：`executor/{value,value_ref}.rs`（Date/Timestamp 变体与 equals/gt/lt/ge/le/Hash/Display/as_value_ref/to_value/lt_agg 臂）、`executor/sort.rs`（compare_values 两臂）、`executor/plan.rs`（to_schema_column 两臂）、`executor/predicate.rs`（CastType 两变体 + CAST 矩阵 8 臂 + evaluate_ref Copy 直回 ×5 处 + 比较严格性守卫 + `BinaryArithExpression`/`ArithOp`）、`executor/function.rs`（evaluate_ref Copy 直回）、`executor/insert.rs`（逐列 coerce + key_value_type_name）、`executor/update.rs`（SET coerce 先于写入 + Step 3/7 消费 coerce 值）、`executor/mod.rs`（pub(crate) mod datetime + predicate 导出）、`storage/page_format/tuple.rs`（TAG_DATE/TAG_TIMESTAMP + compute/serialize/deserialize ×2 + storage ColumnType）、`storage/catalog.rs`（COL_TAG 两臂）、`storage/error.rs`（InvalidDateTime）、`parser/ast.rs`（两放行清单 +TypedString/BinaryOp）、`parser/planner/ddl_dml.rs`（convert_data_type Result 化 + extract_insert_values/build_update TypedString 臂）、`parser/planner/expression.rs`（build_expression TypedString/BinaryOp 臂 + CAST 目标映射 + convert_cast_data_type 日期族）、`pipeline.rs`（value_to_json 两臂）、`network/pg_messages.rs`（OID 1082/1114 + 尺寸 + DA5 文本编码）、`cli/lifecycle.rs`（dump 类型化字面量 + csv_value/column_type_sql 两臂）
- 校准：`tests/expression_e2e_test.rs` cast_unknown_target_type_rejected 示例类型 DATE→TIME（见 Deviations 6）

**Deviations from Plan**

1. **穷尽匹配补臂超出契约点名面（T2）**：双 `ColumnType` 加变体使 11 处非穷尽 match 编译失败（契约只列 tuple/catalog/plan/lifecycle 主要目标）。全部为加性语义必需臂：evaluate_ref Copy 直回 ×5（predicate/function，恰为 T5「Copy 直回」目标行为）、key_value_type_name ×2（错误文案类型名）、PG row_description/data_row ×3（OID 1082/1114 + DA5 文本编码）、value_to_json（T5 目标行为）。csv_value/column_type_sql/to_schema_column 同理在 T2 落臂（T3/T5 目标行为），验收在 T3/T5 完成。
2. **extract_qualified_columns 同门放行（T4）**：契约只列 extract_columns，但 query.rs:394 对非 JOIN 查询也无条件调用 qualified 提取，BinaryOp 在此被拒（实测「Unsupported statement type」）。两清单同步放行；表达式项 + JOIN 显式拒绝由 planner 路由层统一裁决（与 CASE/CAST 同型，query.rs:484），拒绝面未 loosened。
3. **BinaryArithExpression 新节点（T4）**：design.md 行 6 记载「build_expression 已有 BinaryOp 算术臂（expression.rs:495）」系误记（:495 实为 build_where 谓词臂），T4 契约 GREEN 与 no-from-select R3 场景（`SELECT id + 1 FROM t` → 2,3）要求 WITH-FORM 算术可达。新增最小算术节点（沿用 CASE/CAST 表达式节点模式；语义：NULL 传播、严格数值面、Add/Div 复用既有 `Value::add/div` 聚合约定——Int/Int 截断除、零除 Null），目标行为由 spec 场景锁定，非 Act 自创设计。
4. **WHERE 比较腿算术随之可达（T4 副作用）**：共享 build_expression 编译臂使 `WHERE id + 1 > 3` 从拒绝变为可达（此前落入 UnsupportedExpression 兜底）。行为扩展非回归；如需收紧为 SELECT-only 需上下文旗标（更大改动），交 Plan Review 裁定。
5. **R4-S2 跨类型比较错误的实现位置（T5）**：design D7「equals 跨族 → false」与 spec R4 场景「`WHERE d = '2024-01-15'` 显式类型错误」矛盾（T4 契约 Preserve 亦写「保持跨族错误」）。以 spec 为验收权威：在 `ComparisonPredicate::evaluate_ternary` 加谓词层守卫（任一侧日期族且非同变体 → TypeMismatch，Null 短路保持）；`Value::equals` 跨族 false 原样（D7 字面成立），MIN/MAX/排序兜底不受影响，既有五类型互比行为零变化。
6. **expression_e2e_test 1 处校准（T5）**：cast_unknown_target_type_rejected 以 `CAST(v AS DATE)` 作「未知 CAST 目标」示例——spec R5 合法化 DATE 目标后该夹具失效（BH-1 同型先例）。示例改为仍被拒绝的 TIME，断言意图（计划期拒绝未知目标）不变；R7「既有全量零修改」因此破例此一处，请 Plan Review 在 spec R7 增补校准段（参考 key-column-type-conformance BH-1 先例）。

**Blocker Handoff**

None required。

**Blocker Resolution**

（未发生阻塞。）

**Self-Review**

- Plan compliance: T1–T5 契约逐项覆盖（Targets/Preserve/Forbidden 无违反；InvalidDateTime/ParseError 文案、MS16 预检顺序、WAL/恢复/索引层零改动均遵守）；偏差 1–6 全部记录。
- Full diff reviewed: 是——19 个修改文件 + 2 新文件全量 diff 审查（含跨任务交互：INSERT coerce 先于 MS16 预检、UpdateExecutor Step 3/7 消费 coerce 值、dump zip 行列对齐依赖 SELECT * 恒等投影、谓词守卫对 IN 脱糖链生效）。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: (a) BinaryArithExpression Int 算术沿用既有 add/div 原始运算约定（debug 溢出 panic/release 回绕——与 SUM/AVG 既有行为一致，I043 同族面 Iteration 001 顺带）；(b) trunc_ts 的 Hour/Minute/Second 变体 Iteration 001 T7（date_trunc）消费，暂以注释性 `#[allow(dead_code)]` 标注；(c) 测试夹具 CSV ragged row 曾在本 Cycle 内修正（测试问题非产品缺陷）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 单测 | `cargo test --lib datetime` | `test result: ok. 31 passed; 0 failed`（RED 17 failed → GREEN） | datetime.rs 全函数（闰年/月末/往返/边界/截断/coerce 矩阵） | PASS |
| T2 lib | `cargo test --lib` | `265 passed; 0 failed`（RED 70 编译错见证） | Value/ValueRef/tuple/catalog/sort 全部单测 | PASS |
| T3/T4/T5 e2e | `cargo test --test datetime_type_test` | `16 passed; 0 failed`（RED 2→7→4 failed 分组见证） | datetime-type-system R1–R6 行为域 | PASS |
| 相邻套件 | expression_e2e/projection/planner/pushdown/scalar_function/subquery | `24+36+6+16+28+28 passed`（各 0 failed） | 表达式/投影/计划/下推/函数/子查询回归 | PASS |
| 全量回归 | `cargo test --no-fail-fast` | `passed=987 failed=0`（936 基线 + 51 新增；1 处校准见偏差 6；I041 已知竞态曾在一次中间运行偶发、单跑与复跑均绿） | 全部 74 bins | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` | 均 0（exit 0） | 全 workspace | PASS |
| OpenSpec | `openspec validate 2026-09-23-ms13-analytics-functions` | `Change ... is valid` | change 结构 | PASS |
| CLI 探针 | `rtsql ev "SELECT * FROM ev"`（json） | `{"columns":["id","d","ts"],"rows":[[1,"2024-02-29","2024-01-15 10:30:00.123456"]]}` | json 渲染 DA5 字符串形态 | PASS |

**Persisted Evidence**

None required（Mode none：全部验证可低成本重跑，无一次性环境与 Issue 现场）。

**Experience Candidates**

None。

**Remaining Issues**

None（I041 resolve env 测试竞态为已登记既有项，本次全量运行曾复现一次偶发假失败，单跑/复跑均绿，不新增登记）。

**Commit or Diff Reference**

未提交（待用户触发）；对照基线 7364bc9，工作区另含本 change 实施前的 MS09 收尾 docs sync 增量（非本 Cycle 改动）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查覆盖全部 19 修改 + 2 新增文件 diff 与 16 e2e 场景映射（代码级证据见下）；无阻塞项。非阻塞 finding：

- **F1（Minor，Dev 4）**：WHERE 侧算术比较腿（`WHERE id + 1 > 3`）随共享编译臂解锁可达，无 spec 场景见证。与 WITH-FORM 算术解锁同一通路（后者已入 no-from-select R3 场景），收紧需上下文旗标（更大改动面），不值得。行为正确、语义与既有严格类型面一致；建议随本 change 收尾归档时在 `sql-expression-evaluation` 域补见证（归档合并时的语料库事项，非本 Cycle 返工）。
- **F2（Minor）**：`BinaryArithExpression` 的 doc 注释称服务「Iteration 001 datetime INTERVAL legs」，但其数值严格守卫按 design D11 本就不承载 INTERVAL——Iteration 001 应经独立 `IntervalArithExpression` 分流（Cycle 001 契约已明确）。注释表述误导性轻微，不构成行为问题。
- **F3（Minor，测试现象）**：Plan Review 独立复跑一次出现单例失败、复跑通过，特征与已登记 I041（resolve env 竞态，MS16 记载约 1/6 假失败源）一致；Act Response 亦记录同现象（单跑/复跑均绿）。用户裁定测试结论采信 Act 报告。不构成 Acceptance 缺口，不新登记（既有项）。

**Deviation Classification**

- Dev 1（穷尽匹配补臂 ×11 超契约点名面）→ **PLAN-OMISSION**（机械必需臂，全部服务 T3/T5 目标行为；契约「主要落点」表述未穷举编译器强制面）。
- Dev 2（extract_qualified_columns 同门放行）→ **PLAN-OMISSION**（Plan 漏列 query.rs:394 对非 JOIN 查询也无条件调用的第二提取门；Act 实测发现并同步放行，拒绝面由 planner 路由层统一裁决——与 CASE/CAST 同型，正确）。
- Dev 3（BinaryArithExpression 新节点）→ **PLAN-INVALID**（design D13 段落误记「build_expression 已有 BinaryOp 算术臂（expression.rs:495）」——:495 实为 build_where 谓词臂，规划调查笔误）。Act 补救正确：新增最小算术节点（CASE/CAST 表达式节点同型），行为由 no-from-select R3 场景锁定（`SELECT id + 1 FROM t` → 2/3），非 Act 自创范围。
- Dev 5（R4-S2 跨类型比较错误实现位置）→ **PLAN-INVALID**（design D7「equals 跨族 false」与 spec R4 场景「显式类型错误」表述矛盾——Plan 设计歧义）。Act 以 spec 为验收权威在 `ComparisonPredicate::evaluate_ternary` 加日期族同变体守卫：NULL 短路保持、Int/Float 跨族互比零变化、`Value::equals` 跨族 false 字面成立（MIN/MAX/排序兜底不受影响）——歧义消解方式正确。
- Dev 6（expression_e2e 1 处校准）→ **BASELINE-CHANGED**（夹具依赖「DATE 为未知 CAST 目标」的旧基线行为，R5 合法化后必然失效，BH-1 同型）。已按 Act 请求在 delta spec R7 增补校准段（本次 Review 写入）。
- Dev 2/Dev 4 中行为扩展面（WHERE 算术）→ 见 F1。

**Acceptance Gaps**

None。datetime-type-system R1–R7 全部场景有对应见证（16 e2e + tuple/catalog/value/sort 单测 + 全量绿 + clippy/fmt/validate 0 + CLI json 探针）；T1–T5 契约 Targets/Required behavior 逐项核对实现无缺失；Preserve/Forbidden（MS16 预检顺序、WAL/恢复/索引零改动、比较严格面、既有五类型行为）经 diff 审查确认遵守。

**Convergence**

N/A（首次 Review；无既有 gap 比较）。

**Evidence**

- 代码独立审查：本 Review 全部 diff 逐文件读取（datetime.rs 562 行全读——Hinnant 算法逐行核对、`DAYS_0001_TO_1970` 由 `-days_from_civil(1,1,1)` 推导非手抄、`div_euclid/rem_euclid` 处理负微秒、`trunc_ts` 月/年日历地板正确、`coerce_datetime_write` 与 D6 逐条一致；insert coerce 置于 MS16 预检前零副作用；update coerce 先于 Step 2 读、Step 3/7 消费 coerce 值；谓词守卫位置在 NULL 短路后；dump zip 行列对齐依赖 SELECT * 恒等投影成立；csv_value 空字段经既有 `_ => Null` 默认→NULL）。
- 采信（覆盖范围未失效，git 基线检查一致）：Act Response Verification Evidence 表——全量 987 passed / 0 failed（936 基线 + 51 新增）、clippy/fmt/validate 0、`rtsql ev "SELECT * FROM ev"` json 探针 DA5 形态。用户裁定（2026-09-23）：测试结论采信 Act 报告，不再重复运行。
- spec 校准：datetime-type-system R7 校准段已写入（Dev 6 闭环）。

**Follow-up Decision**

Acceptance 全满足、无阻塞 finding、六项偏差全部非阻塞且已闭环（校准段 F1/F2 归档期处理）——接受本 Cycle，Iteration 000 完成。F1 的 `sql-expression-evaluation` 见证补齐属语料库归档事项，不创建返工。

**Iteration Plan Update**

None（Iteration Map 不变；F1/F2 为归档期注记，非计划变更）。

**Next Cycle**

None（无 rework/replan）。

**Next Iteration**

`../001-datetime-functions-groupby/000-initial.md`（已按 Map 展开，Status: draft 待 Gate 2）
