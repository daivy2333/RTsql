## MODIFIED Requirements

### Requirement: 生命周期子命令 dump 与 restore

（既有主体保持——PK 渲染规则、数据链预检、restore 语义。）本次追加列类型名保真：`schema` 与 `dump` 的 DDL 列类型 SHALL 渲染标准长名——`INTEGER`、`FLOAT`、`BOOLEAN`、`DATE`、`TIMESTAMP`、`TEXT`（不再输出引擎内部短名 `INT`/`STRING`/`BOOL`）。dump→restore 往返与多代 dump/restore 幂等 SHALL 保持（解析器接受长名）。

#### Scenario: 类型名渲染与声明一致

- **GIVEN** `CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, ok BOOLEAN, score FLOAT)`
- **WHEN** `rtsql schema t` / `rtsql dump t`
- **THEN** DDL 输出 `INTEGER`/`TEXT`/`BOOLEAN`/`FLOAT`（失败边界——修复前：渲染 `INT`/`STRING`/`BOOL` 短名）

#### Scenario: dump→restore 往返与多代幂等

- **GIVEN** 同上库 dump
- **WHEN** restore 到新库后再 dump
- **THEN** 类型名与数据逐字节幂等

### Requirement: 锁等待 busy-timeout 通道

（既有主体保持——三通道、默认 5000ms、flag 优先、`:memory:` 接受忽略、`delete` 探测不等待。）本次修正 env 通道非法值报错面：经 `RTSQL_BUSY_TIMEOUT` 设置非法值（非 u64）SHALL 在任何开库前 exit 2，报错文案 SHALL 点名环境变量 `RTSQL_BUSY_TIMEOUT` 与实际值（不再只指向 `--busy-timeout` flag）；flag 显式值优先语义与退出码 SHALL 不变。`RTSQL_KEY` / `RTSQL_WAL_CHECKPOINT_BYTES` 同族 env 校验报错 SHALL 同样点名各自 env 名（如有同病一并收敛）。

#### Scenario: env 非法值报错点名环境变量

- **WHEN** `RTSQL_BUSY_TIMEOUT=abc rtsql demo "SELECT 1"`
- **THEN** exit 2，报错含 `RTSQL_BUSY_TIMEOUT` 与 `abc`（失败边界——修复前：`error: invalid value 'abc' for '--busy-timeout <MS>'`——env 设置者被误导去查不存在的命令行参数）

#### Scenario: flag 优先与既有锁矩阵零回归

- **WHEN** flag 与 env 同时设置（flag 合法 env 非法 / 两者均合法不同值）；既有锁等待/超时/信号中断用例
- **THEN** flag 优先、锁矩阵行为与退出码不变

## ADDED Requirements

### Requirement: 多语句 JSONL 与 DDL affected_rows 输出形态（锁定）

多语句输入以 `--format json` 执行时，输出 SHALL 为逐语句一个独立 JSON 对象（JSONL 流，含既有 fail-fast 序号定位）；DDL 语句成功 SHALL 输出 `{"affected_rows":0}`。两者为文档化设计形态（2026-10-13 用户裁定维持现状），以测试与文档锁定；退出码与渲染语义 SHALL 零变化。

#### Scenario: 多语句 JSONL 形态

- **GIVEN** 库与两条查询语句以 `;` 连接
- **WHEN** `rtsql demo --format json "SELECT 1; SELECT 2"`
- **THEN** stdout 两行独立 JSON 对象（每行可独立解析）

#### Scenario: DDL affected_rows 形态

- **WHEN** `rtsql demo --format json "CREATE TABLE t (id INTEGER)"`
- **THEN** 输出 `{"affected_rows":0}`、exit 0

#### Scenario: help 文本无内部开发标记

- **WHEN** `rtsql --help` / `rtsql import --help`
- **THEN** 帮助文本不含内部任务编号（`MS2x` 类）与提案标记（`proposal`）（失败边界——修复前：`--busy-timeout` help 含「MS28 T1：」、`--readonly` help 含「MS26 T1：」「proposal 假设 10」）
