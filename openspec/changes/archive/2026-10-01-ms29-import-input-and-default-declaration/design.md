# ms29-import-input-and-default-declaration — Design

## D1：stdin 通道复用 `-` 文件位（无新 flag）

`import` 的第三个位置参数（文件位）接受 `-` 表示 stdin——与 `restore` 的 `read_restore_input`（lifecycle.rs:573）同构。实现为 CSV 读取分支：`file == "-"` → `csv::Reader::from_reader(std::io::stdin().lock())`，否则 `csv::Reader::from_path`。既有 records 预收集语义（`reader.records().collect()`，lifecycle.rs:668）对 stdin 同样成立（读到 EOF，与 restore 的 stdin 语义一致）；行定位 `row k of n`、RFC4180 解析、逐条 auto-commit 全部零改动。备选方案（新增 `--stdin` flag）被否：与 restore 先例不一致，且多一个 flag 无语义增益。

## D2：自动建表数据流（get_table 失败 → 推导 → DDL → 复用既有插入循环）

`import_csv` work 闭包内：

1. `db.get_table(&table)` 成功 → 既有路径逐字节不变（表头匹配、csv_value 转换、fail-fast）。
2. 失败（表不存在）→ 自动建表路径：
   a. 读 CSV（文件位或 stdin）取 `headers` + 全量 `records`（与既有预收集一致）；
   b. 逐字段猜测（D3）构造 `Vec<Vec<Value>>`；
   c. `infer_ctas_column_types(&headers, &rows)`（pipeline.rs:777，可见性扩为 `pub(crate)`，矩阵逻辑零改动）→ `Err(CtasColumnTypeConflict(msg))` → `ExitStatus::General`（exit 1，文案含矩阵 Display 的「列名 (类型, 类型)」）——推导冲突在收齐数据后、任何写入前判定，零副作用结构性成立；
   d. 生成 DDL：`CREATE TABLE {quote_ident(table)} ({quote_ident(col)} TYPE, ...)`——类型名映射 INT/FLOAT/STRING（`create_table_sql` 同款大写形态；`StorageColumnType = page_format::ColumnType`，pipeline.rs:18）；无主键（D4）；经 `db.execute_sql` 执行 → 失败走既有 SQL 失败通道（`ExitStatus::Sql`，exit 3，保留名/重名视图/重复列名等计划期守卫自然生效）；
   e. 后续逐行插入**复用既有循环零改动**：`col_index` 为恒等映射（表头序 = schema 序），转换用列类型直接取推导结果（不重取 catalog——规避大小写归一化二次查询），`csv_value` 消费 Int/Float/String 三类（Bool 列在自动建表形态不可达）。

备选（直接调 `table_manager.create_table_with_constraints`）被否：绕过 planner 校验/名字归一化/视图重名守卫，且 dump/schema 渲染一致性需自行保证；DDL 通道一次复用全部既有设施。

## D3：逐字段猜测语义（用户裁定：仅 INT/FLOAT/STRING）

纯函数 `csv_field_guess(field: &str) -> Value`：空字段 → `Value::Null`（参与任意类型，不制造冲突）；可解析 i64 → `Value::Int`；可解析 f64 → `Value::Float`；其余（含 `true`/`false`、日期文本）→ `Value::String`。推断数据源 = 全部数据行。与既有 `csv_value` 的 Float 解析同口径（`f64::from_str` 接受 `inf`/`NaN` 等，既有 Float 转换语义照常，不特判）。推断为 STRING 的列空字段插入空串、Int/Float 列插入 NULL——与既有表导入的空字段规则一致（proposal 默认假设 4）。

## D4：自动建表沿用引擎「首列隐式主键」语义（replan 修订，2026-10-01 阻塞审计）

原设计「DDL 不声明 PRIMARY KEY 即落无键行」基于错误调查事实——`CreateTableExecutor::execute`（`src/executor/create_table.rs:74-83`）对 `primary_key: None` **默认取第一列作主键**（引擎不存在表级无主键概念；「无键行」是行级语义：PK 值为 NULL 的行不入索引）。修订：

- 自动建表 DDL 不写 PRIMARY KEY，新表自然落**首列隐式主键**——与 `create-table-as-select` 物化表同款引擎语义；`rtsql schema`/dump 渲染首列 `PRIMARY KEY`，restore 经执行器同款默认重建，往返一致。
- 失败面收窄：仅**首列推导为 INT 且数据含重复值**时，逐条 INSERT 于首个重复值处 `DuplicateKey` fail-fast（exit 3，失败前已提交行保留——与既有转换失败同语义）；首列 FLOAT/STRING 行落非键行语义，重复值合法。
- 备选（真无主键表）结构性不可行：dump DDL 无法表达无主键，即使直调 `create_table_with_constraints` 绕道建出，restore 时执行器重默认首列必坏往返——需引擎级新特性（DDL 表达力/执行器默认/dump/restore/schema 渲染联动），远超本 change 定位，如需另立路线。

## D5：`:memory:` import 豁免面

- `lifecycle_db_target`（mod.rs:339）对 `Command::Import` 放行（`:memory:` 守卫 :247-253 不再命中 import）；
- `execute_command` 的 Import 分派臂处理 memory 分支：`db == MEMORY_DB && args.key.is_some()` → `ExitStatus::Usage`（文案对齐主命令 `:memory:` × `--key` 拒绝，mod.rs:374）；
- `import_csv` 增 memory 路由：memory 形态跳过 `resolve_db_path`/存在性校验，经 `execute_command_inner_mode(..., in_memory=true, ...)` 开库（mod.rs:510，memory 臂 `Database::open_in_memory()`）；文件库形态现有路径零变化。`--readonly` 组合由既有全局守卫拒绝（mod.rs:257），无需新代码。
- 备选（在 `lifecycle_db_target` 返回后特判）等价，落点非实质。

## D6：声明期校验/归一单点 `validate_default_value`

新纯函数（ddl_dml.rs 或其可见 helper）：`validate_default_value(col_name, col_type, value) -> Result<Option<Value>, PlanError>`（`None` = 原样保留，`Some(v)` = 归一后）：

- `Value::Null` → `Ok(None)`（NULL 性由 NOT NULL 门裁决，豁免）；
- `(Float, Value::Int(n))` → `Ok(Some(Value::Float(n as f64)))`（写入类型门升格同款，归一持久化——catalog 默认值类型恒一致，dump 渲染跟随为 `2.0`）；
- `(Date | Timestamp, _)` → 调 `coerce_datetime_write(value, col_type)`（datetime.rs:268，`pub(crate)`，写入边界强制解析同源）：`Ok` → `Ok(None)`（String 形态保留目录既有通路）；`Err` → `Err(DefaultTypeMismatch(...))`；
- 其余同型 → `Ok(None)`；异型 → `Err(DefaultTypeMismatch(...))`（点名列名、期望类型名与实际值类型名——`column_type_name` 同款大写形态）。

消费点两个：`build_create_table`（ddl_dml.rs:868 后，column_defs 装配完成的单循环——在 PK/UNIQUE 策略面之后、返回前）与 `build_alter_table` AddColumn（:1099 后，`column_type` 与 `default_value` 交汇处）。两消费点共享同一函数＝同规则面承诺的结构保证。

## D7：TypedString 臂（dump→restore 往返修复）

`extract_default_value`（ddl_dml.rs:722）补 `Expr::TypedString { data_type, value }` 臂：`DataType::Date` / `Timestamp`（含 `Timestamp(_, None)`）→ 构造 `Value::String(value)` 经 `coerce_datetime_write` 验证解析通过后返回 `Value::String(value)`（目录存储形态与 String 字面量声明一致，D6 双重验证无害）；其余 data_type → `Err(UnsupportedValue)`（既有语义不变）。dump 渲染（`typed_datetime_literal`）与写入门 coerce 通路零改动——restore 链路 `CREATE TABLE … DEFAULT DATE '…'` → TypedString 臂 → String 目录形态，与手写 String 声明完全同态。

## D8：错误面 `PlanError::DefaultTypeMismatch(String)`

新变体，消息即全文（`CtasColumnTypeConflict(String)` 同型），Display 含列名/期望类型/实际类型；SQLSTATE 分类 58000（沿 `CtasColumnTypeConflict` 先例——DDL 语义冲突类；42601 属语法类不适用）。CLI 主命令既有计划错误映射 → exit 3。若实施中判定复用既有 `InvalidConstraint(String)` 分类等价，属非实质选择（Response 记录）。

## D9：测试放置与校准面

- import 输入面：新文件 `tests/import_input_surface_test.rs`（stdin 管道、自动建表矩阵、冲突拒绝、`:memory:` 组合、缺库不变；子进程 e2e，独立文件避免 cli_test 时延膨胀）。
- 声明面：新文件 `tests/default_declaration_test.rs`（建表/加列拒绝矩阵、升格归一、日期解析、TypedString 往返、重开保真）+ `tests/planner_test.rs` 计划期单元若干。
- 校准：`tests/cli_test.rs::test_import_missing_table_or_db` 表缺失臂按自动建表语义校准（原拒绝断言由新矩阵用例承接，断言意图不丢失）；其余 import 用例零校准。
- RED 见证：T1（stdin `from_path("-")` 失败 exit 1）、T2（`table t does not exist` exit 1）、T3（`:memory:` 守卫 exit 2）、T4（BOOL DEFAULT 1 建表成功＝分裂形态可达）、T5（proposal 实证序列：restore `Unsupported value type` exit 3）、T6（ADD COLUMN BOOL DEFAULT 1 成功）。

## D10：planner 重复列名计划期守卫（replan 增入，T8）

`build_create_table`（ddl_dml.rs）在 column_defs 装配完成后（T4 声明门同位置）增重复列名检查：列名已小写归一（`:859`），对归一后同名判定重复，首个重复即 `Err(PlanError::InvalidQuery(format!("column '{}' specified more than once", name)))`（复用既有 `InvalidQuery` 变体——CTAS 输出列重复同面，不新增变体与分类表行；文案措辞非实质）。效果：

- 根因修复既有实质缺陷（2026-10-01 实证：`CREATE TABLE dup ("a" INT, "a" INT)` rc=0、schema 渲染双 `PRIMARY KEY`、dump→restore 必触发 `MultiplePrimaryKey`）——「静默接受但损坏」缺陷类按项目纪律收口；Act 登记的 Issue 候选随之在本 change 内闭环，无需落账。
- 自动建表重复表头场景经 `execute_sql` 走既有 SQL 失败通道 exit 3（`sql-write-surface` ADDED Requirement「CREATE TABLE 重复列名计划期拒绝」；import 场景 `exit 3` 维持）。
- 范围边界：只守 `CREATE TABLE` 列定义；CTAS 输出列重复既有拒绝不变；`ALTER TABLE ADD COLUMN` 与既有列同名冲突是否存在守卫未调查（本 change 不扩围，留待缺陷台账按证据裁定）。

## 实施顺序与依赖

T1 → T2 → T3（同一函数面自内向外：stdin 通道是自动建表 stdin 形态的前置；`:memory:` 路由消费 T1/T2 成果）→ T4 → T5 → T6（共享 D6/D7 helper，T5 依赖 T4 的错误变体）→ T7（消费全 change 语义）。000 与 001 无代码耦合（不同故障域），仅文档依赖两轮语义。

**Replan 修订（2026-10-01）**：T8 增入 Iteration 000、位于 T2 收尾之前（T2 两场景校准的依赖前置——重复列名场景消费守卫、首列重复值场景按 D4 修订语义重校准）；T1/T3 已完成（000-initial Act Response，验证结论采信继承）。000 与 001 无代码耦合不变。
