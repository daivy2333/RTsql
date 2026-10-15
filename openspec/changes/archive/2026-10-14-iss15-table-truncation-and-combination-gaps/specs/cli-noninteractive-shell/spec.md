## MODIFIED Requirements

### Requirement: 输出格式四态

CLI SHALL 支持 `--format table|json|csv|tsv`；未指定时 TTY stdout 默认 table、非 TTY 默认 json。CSV/TSV SHALL 用 RFC 4180 风格转义（引号包裹含分隔符/引号/换行的值，引号翻倍）。NULL 的文本渲染 SHALL 为空（与 PG DataRow 语义一致）。

本次追加 table 格式大单元格降级语义：任一单元格（含表头）显示字符数超过固定上限（64）时，table 渲染 SHALL 将该单元格截断为前 61 字符 + `...`（总显示宽度恰 64），列宽 SHALL 按截断后文本计算；table 渲染 SHALL 对任意单元格长度 exit 0，不因单元格长度 panic（修复前：≥65536 字符触发 std::fmt 动态宽度上限 panic，exit 101）。截断 SHALL 按字符（chars）边界执行，多字节字符不产生部分字符。`json`/`csv`/`tsv` SHALL 保持全值保真（不截断、逐字节不变）。

#### Scenario: TTY 默认表格

- **GIVEN** stdout 为 TTY（集成测试中用 pty 或以显式 --format 验证渲染函数）
- **WHEN** 未指定 `--format` 执行 SELECT
- **THEN** 输出对齐表格（含表头与行分隔）

#### Scenario: 非 TTY 默认 JSON

- **GIVEN** stdout 为管道（非 TTY）
- **WHEN** 未指定 `--format` 执行 SELECT
- **THEN** 输出 `{"columns":[...],"rows":[[...],...]}` 形状的合法 JSON

#### Scenario: CSV 转义

- **GIVEN** 某字符串值含逗号、引号或换行（如 `a"b,c`）
- **WHEN** `--format csv` 渲染
- **THEN** 该字段以引号包裹并正确转义（`"a""b,c"`），可被标准 CSV 解析器还原

#### Scenario: TSV 转义

- **GIVEN** 某字符串值含制表符、换行或引号
- **WHEN** `--format tsv` 渲染
- **THEN** 字段内制表符/换行转义（`\t`/`\n`），字段不以引号包裹，行结构可还原

#### Scenario: DML 与 DDL 的输出

- **GIVEN** INSERT/UPDATE/DELETE/CREATE TABLE 语句
- **WHEN** 以任意格式执行
- **THEN** 输出受影响行数（table/json：`AffectedRows` 语义；csv/tsv：同值单字段），退出码 0

#### Scenario: 超限单元格截断且不 panic

- **GIVEN** 某 STRING 值 100,000 字符（≥65536）
- **WHEN** `--format table` 渲染（SELECT 或分析子命令）
- **THEN** exit 0，该单元格显示前 61 字符 + `...`（共 64 字符）（失败边界——修复前：exit 101，panic `Formatting argument out of range`）

#### Scenario: 未超限与截断边界

- **GIVEN** 单元格分别为 64 字符与 65 字符
- **WHEN** `--format table` 渲染
- **THEN** 64 字符原样显示；65 字符截断为 64 显示字符（前 61 + `...`）；既有 ≤64 字符输出逐字节不变

#### Scenario: 三管道格式保真零回归

- **GIVEN** 同一大值
- **WHEN** `--format json` / `--format csv` / `--format tsv` 渲染
- **THEN** 输出含完整原值，长度逐字节一致（不截断）

## ADDED Requirements

### Requirement: 大值行集输出组合（锁定）

主命令 SELECT 与分析子命令 stats/sample/profile 对含超限大值（≥65536 字符）的行集 SHALL 全格式可用：`table` exit 0 且超限单元格截断；`json`/`csv`/`tsv` 全值保真。stats 的 String 列 min/max、sample 行渲染、profile top-k 等携带原值的输出路径 SHALL 同受 table 截断保护、json 保真。本 Requirement 锁定 2026-10-14 组合空格清偿后的输出组合行为。

#### Scenario: 大值 × 主命令四格式

- **GIVEN** 表含 100,000 字符 STRING 值（E8 大值旅程表）
- **WHEN** `SELECT body FROM docs` 分别以四格式执行
- **THEN** json/csv/tsv 全值保真；table exit 0、数据行恰 64 字符以 `...` 结尾（失败边界——修复前 table exit 101）

#### Scenario: 大值 × 分析子命令（用户实测 panic 路径）

- **GIVEN** 表含 70,000 字符 STRING 列值（stats min/max、sample 行渲染、profile top-k 均携带原值）
- **WHEN** stats/sample/profile 以 `--format table` 执行
- **THEN** 三命令 exit 0，超限值截断显示；stats 以 `--format json` 执行时 min/max SHALL 携带完整原值（失败边界——修复前 exit 101，与用户 2026-10-14 实测一致）

### Requirement: import 与分析子命令非默认运行形态（锁定）

`import --csv` 目标为加密文件库时 SHALL 经 `--key` 打开写入：对钥 exit 0 且导入数据可查；错钥 exit 5（与主命令密钥错误面一致）。stats/sample/profile 对加密文件库 SHALL 经 `--key` 可用（exit 0）。stats/sample/profile 与 `--readonly` 组合 SHALL Usage exit 2 且报错点名 `--readonly` 归属主命令（与 schema/dump/delete/restore/import 同一拒绝面，`src/cli/mod.rs` 通用拒绝臂）。本 Requirement 锁定既有通道与拒绝面的组合行为（此前零见证）。

#### Scenario: import × 加密库对钥

- **GIVEN** `--key pw` 创建的加密库含目标表，CSV 文件就绪
- **WHEN** `rtsql --key pw import <enc-db> <table> <file> --csv`
- **THEN** exit 0，`--key pw` 查询导入行数正确（数据真实落库可查）

#### Scenario: import × 加密库错钥

- **GIVEN** 同上加密库
- **WHEN** `rtsql --key wrong import <enc-db> <table> <file> --csv`
- **THEN** exit 5，零行写入

#### Scenario: 分析子命令 × 加密库

- **GIVEN** `--key pw` 创建并写入的加密库
- **WHEN** `rtsql --key pw stats <enc-db> <table> --format json`
- **THEN** exit 0，统计输出正常

#### Scenario: 分析子命令 × --readonly 拒绝

- **WHEN** `rtsql --readonly stats|sample|profile <db> <table>`
- **THEN** 三者均 Usage exit 2，报错点名 `--readonly` 仅属主命令（失败边界——此前拒绝矩阵无此三项见证）
