# ms29-import-input-and-default-declaration — Tasks

## 任务清单

| Task | 目标 | Iteration | Requirement 映射 |
|---|---|---|---|
| T1 | import stdin 通道：文件位 `-` → `csv::Reader::from_reader(stdin)`（对齐 `read_restore_input` 先例）；records 预收集/RFC4180/逐条 auto-commit/输出语义零变化；lib+CLI 测试（stdin 导入既有表、stdin 自动建表形态随 T2 见证） | 000 | cli-noninteractive-shell import MODIFIED（stdin 场景） |
| T2 | 免建表导入：`get_table` 失败 → 自动建表路径（读全量 CSV → `csv_field_guess` 逐字段猜测〔空→Null、i64→Int、f64→Float、其余 String〕→ `infer_ctas_column_types` pub(crate) 复用 → 冲突 General exit 1 点名零副作用 → DDL 经 `quote_ident` 生成 `execute_sql` 执行〔失败 Sql exit 3〕→ 恒等列映射复用既有插入循环零改动）；自动建表不显式声明主键——沿用引擎「未声明 PK → 首列隐式主键」语义（replan 修订：首列 INT 重复 DuplicateKey fail-fast、FLOAT/STRING 首列重复合法，design D4）；既有表路径逐字节不变；矩阵测试（推导/提升/全 NULL 列/零数据行/冲突/隐式主键 INT 重复 fail-fast + FLOAT/STRING 重复合法/保留名/重复列名拒绝）+ `test_import_missing_table_or_db` 表缺失臂校准 | 000 | cli-noninteractive-shell import MODIFIED（自动建表全部场景） |
| T3 | `:memory:` import 豁免：`lifecycle_db_target` Import 放行 + Import 分派臂 `--key` × `:memory:` Usage exit 2（文案对齐主命令）+ `import_csv` memory 路由（`execute_command_inner_mode(in_memory=true)` 跳过 resolve/exists）；`--readonly` 全局守卫自然覆盖；其余生命周期子命令 `:memory:` 拒绝零变化 | 000 | cli-noninteractive-shell import MODIFIED（`:memory:` 三场景） |
| T4 | CREATE TABLE 声明期 DEFAULT 校验/归一：新 `validate_default_value` 单点（Null 豁免；Float×Int→升格归一 `Float(n as f64)`；Date/Timestamp 经 `coerce_datetime_write` 验证〔Err→拒绝〕；异型→拒绝）消费于 `build_create_table`（column_defs 装配后）；新 `PlanError::DefaultTypeMismatch(String)`（CtasColumnTypeConflict 同型、SQLSTATE 58000、分类表测试行）；拒绝/归一矩阵测试 | 001 | sql-write-surface ADDED（声明期校验/归一场景） |
| T5 | TypedString 臂与 dump→restore 往返修复：`extract_default_value` 补 `Expr::TypedString` 臂（Date/Timestamp → `coerce_datetime_write` 验证后返回 `Value::String`；其余 data_type → 既有 `UnsupportedValue`）；日期族 DEFAULT dump→restore e2e（修复前 RED＝proposal 实证序列）；非日期族 TypedString 拒绝见证 | 001 | sql-write-surface ADDED（TypedString 声明与往返修复场景） |
| T6 | ALTER ADD COLUMN 同款规则：`build_alter_table` AddColumn 消费 `validate_default_value`（NOT NULL 无 DEFAULT 既有拒绝零变化）；ALTER 拒绝/接受矩阵测试 | 001 | sql-write-surface ADDED（ALTER ADD COLUMN 场景） |
| T7 | 文档同步：`rtsql-docs/SKILL.md` 命令行功能目录（import stdin/自动建表/`:memory:` 豁免与拒绝面、DEFAULT 声明语义）+ README 双语对应段核验一致；`openspec validate` | 001 | 文档任务（无 spec 映射，验收 = 文档与实现逐条一致） |
| T8（replan 增入） | planner 重复列名计划期守卫：`build_create_table` column_defs 装配后拒绝归一后同名（复用 `InvalidQuery`，design D10）——修复「重复列名表静默建成且不可 dump 往返」既有缺陷；自动建表重复表头 exit 3；planner_test 守卫矩阵 + 既有建表/CTAS 零回归 | 000 | sql-write-surface ADDED「CREATE TABLE 重复列名计划期拒绝」 |

## Iteration Plan

### Iteration 000: import 输入面（stdin / 免建表导入 / `:memory:` 豁免）

- Tasks: T1, T2, T3, T8（T8 为 2026-10-01 阻塞审计 replan 增入——T2 两场景的依赖前置；T1/T3 已完成且验证结论采信继承）
- Depends on: None
- Stable baseline: `rtsql import <db> <table> <file|-> --csv` 三形态端到端可用——stdin 与文件位等价；目标表不存在时按表头自动建表（类型推导矩阵、冲突点名拒绝零副作用、首列隐式主键语义〔INT 重复 fail-fast / FLOAT·STRING 重复合法〕、重复列名经 planner 守卫 exit 3）；`:memory:` 组合可达（`--key` exit 2、`--readonly` 全局拒绝、进程退出即弃）；文件库缺失维持 exit 1 零文件创建；既有表导入语义与错误面逐字节零回归；普通建表重复列名计划期拒绝（既有缺陷闭环）。
- Verification boundary: `tests/import_input_surface_test.rs`（新，stdin/自动建表/`:memory:` 矩阵）+ `tests/cli_test.rs` import 组零回归（含 `test_import_missing_table_or_db` 校准）+ `tests/planner_test.rs` 守卫矩阵 + 全量分批（M19 纪律）。
- Diagnostic boundary: `src/cli/lifecycle.rs`（import_csv 与 CSV 读取）、`src/cli/mod.rs`（lifecycle_db_target/分派臂）、`src/pipeline.rs`（infer_ctas_column_types 可见性）、`src/parser/planner/ddl_dml.rs::build_create_table`（仅 T8 重复列名守卫单点）。
- Non-goals: DEFAULT 声明面（001）；import 攒批提交（MS22）；Bool/Date CSV 猜测；JSON 输入格式；`:memory:` 其他子命令豁免。

### Iteration 001: DEFAULT 声明期一致（CREATE TABLE/TypedString 往返/ALTER）与文档收尾

- Tasks: T4, T5, T6, T7
- Depends on: Iteration 000（无代码耦合——001 收口后全量分批覆盖两轮；T7 文档消费全 change 语义）
- Stable baseline: 建表/加列声明的 DEFAULT 字面量要么与列类型一致（含 FLOAT 升格归一、日期族解析验证）要么计划期点名拒绝零副作用——「建表成功但应用默认值的 INSERT 必然失败」分裂形态消除；日期族 DEFAULT 的 dump→restore 往返修复（TypedString 臂）；既有 DEFAULT 场景逐字节零回归；SKILL.md 与 README 双语逐条一致。
- Verification boundary: `tests/default_declaration_test.rs`（新）矩阵 + `tests/planner_test.rs` 计划期单元 + `tests/cli_test.rs` DEFAULT/dump 组回归 + 日期族往返 e2e（RED 实证在案）+ 全量分批（M19）+ 文档人工核对与抽样自动执行 + `openspec validate`。
- Diagnostic boundary: `src/parser/planner/ddl_dml.rs`（extract_default_value/build_create_table/build_alter_table/validate_default_value）、`src/parser/error.rs`（新变体与分类表）、`src/executor/datetime.rs`（coerce_datetime_write 消费，零改动预期）。
- Non-goals: 写入类型门运行期语义变更；dump 渲染格式变更；CHECK/FK 强制执行；`DEFAULT <表达式>`（非字面量）支持。

## 平衡审计

- 000（import 任务 + T8）：单一故障域（CLI import 输入面），T1→T2→T3 为同一函数面自内向外递进（stdin 通道 → 建表策略 → 打开模式豁免），拆开任一段都产生「通道存在但组合不可达」的半成品面（如 stdin 无自动建表则 `:memory:` 管道组合不完整）；共享验证边界（import 矩阵）与诊断面（lifecycle.rs/mod.rs）——不拆。T8（replan 增入）是 T2 两场景 Acceptance 的依赖前置（依赖只能指向同一或更早 Iteration），且守卫单点（`build_create_table` 一处检查）不构成独立稳定成果——并入 000 而非独立成轮。
- 001（声明面 + 文档）：T4→T5→T6 共享 `validate_default_value` 单点与同一规则面（同文件 ddl_dml.rs），拆开即产生「部分跨类型仍被接受」的假稳定基线；T7 消费全 change 语义属收尾批（MS28/MS26 收尾批先例）——不拆。
- 两 Iteration 工作量适中（000 中〔T1/T3 已完成、余 T2 校准 + T8 守卫〕、001 小-中），各自形成稳定可验证阶段成果；故障域互异（CLI 面 vs planner 声明面）不合——通过审计（2026-10-01 replan 复审维持）。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| cli-noninteractive-shell import MODIFIED（stdin 通道） | T1 | `import_input_surface_test` stdin 管道用例（既有表 + 自动建表形态） | Covered |
| cli-noninteractive-shell import MODIFIED（自动建表：推导/提升/全 NULL/零行/冲突/首列隐式主键/守卫拒绝/零副作用） | T2 | `import_input_surface_test` auto-create 矩阵 + `test_import_missing_table_or_db` 校准 | Covered |
| cli-noninteractive-shell import MODIFIED（`:memory:` 豁免/`--key` 拒绝/其余子命令维持拒绝） | T3 | `import_input_surface_test` memory 组合矩阵 | Covered |
| cli-noninteractive-shell import MODIFIED（既有表语义零回归：表头匹配/类型转换/fail-fast/锁冲突/引号转义） | T2 | `cli_test` import 组零校准全绿 | Covered |
| sql-write-surface ADDED（CREATE TABLE 声明期校验/归一：跨类型拒绝/FLOAT 升格/日期解析/NULL 豁免） | T4 | `default_declaration_test` 建表矩阵 + `planner_test` 单元 | Covered |
| sql-write-surface ADDED（CREATE TABLE 重复列名计划期拒绝） | T8 | `planner_test` 守卫矩阵（同名/归一后同名/唯一列名零回归）+ `import_input_surface_test` 重复表头 exit 3 | Covered |
| sql-write-surface ADDED（TypedString 声明与 dump→restore 往返修复） | T5 | `default_declaration_test` 往返 e2e（修复前 RED 实证在案） | Covered |
| sql-write-surface ADDED（ALTER ADD COLUMN 同款规则） | T6 | `default_declaration_test` ALTER 矩阵 | Covered |
| sql-write-surface 既有 R1/R2 零回归（DEFAULT 应用/写入门/既有场景） | T4, T5, T6 | `cli_test`/`write_type_conformance_test` 等既有矩阵零回归 | Covered |
| 文档一致性（T7，无 spec 映射） | T7 | SKILL.md/README 逐条人工核对 + `--help` 抽样自动执行 | Covered |
