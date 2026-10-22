# Delta: typed-library-results

## ADDED Requirements

### Requirement: 类型化执行结果

库公共执行入口 SHALL 返回类型化结果：查询语句返回原生 `Value` 行集合与逐列元数据（列名与类型）；DML 语句返回受影响行数；语句错误经 `Result::Err` 返回，错误载荷携带消息与 SQLSTATE 分类，与既有 `Response::Error` 渲染同源。

#### Scenario: SELECT 返回原生值与列元数据

- **GIVEN** 表 `t(a INTEGER, b TEXT)` 含已知行
- **WHEN** 经类型化入口执行 `SELECT a, b FROM t`
- **THEN** 每行为原生 `Value` 变体（Int/Text），逐列元数据的列名与类型和声明一致

#### Scenario: 表达式列元数据

- **GIVEN** 查询含表达式项（如 `a + 1`、`COALESCE(b, 'x')`）
- **WHEN** 执行
- **THEN** 列名按既有输出表头单源；类型按推导矩阵（与 CTAS 推导对齐），推导不可行处按设计定稿的诚实表示

#### Scenario: DML 受影响行数

- **WHEN** 经类型化入口执行 INSERT/UPDATE/DELETE
- **THEN** 返回受影响行数，语义与既有 `AffectedRows` 一致

#### Scenario: 错误载荷同源

- **GIVEN** 触发既有错误面的语句（未知列 42703、未知表 42P01 等）
- **WHEN** 经类型化入口执行
- **THEN** `Err` 的消息与 SQLSTATE 分类和既有 `Response::Error` 同源（边界转换后文本一致）

### Requirement: 核心/协议解耦

核心执行路径（database/pipeline/executor/storage/transaction）SHALL NOT 依赖 `network::protocol::Response`；`Response` SHALL 仅作为 CLI/Server 边界转换词表存在。

#### Scenario: 核心无网络词表依赖

- **GIVEN** 变更后的代码库
- **WHEN** 检查 `src/database.rs`、`src/pipeline.rs`、`src/executor/`、`src/storage/`、`src/transaction/` 对 `network::protocol` 的引用
- **THEN** 无（CLI 与 network 边界除外）

### Requirement: 边界转换输出零变化

CLI table/json/csv/tsv 四格式与 PG/JSON 网络协议输出 SHALL 与本变更前逐字节一致（含错误输出的 SQLSTATE 渲染与退出码）。

#### Scenario: CLI 四格式逐字节等价

- **GIVEN** 同一数据库与同一语句集
- **WHEN** 变更前后分别经 CLI 以四种 `--format` 执行
- **THEN** stdout 逐字节一致

#### Scenario: 协议面零回归

- **GIVEN** 既有 PG/JSON 协议与 CLI 测试基线
- **WHEN** 全量分批回归（M19）
- **THEN** 零失败、零断言校准
