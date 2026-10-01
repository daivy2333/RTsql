# cli-noninteractive-shell Delta — dump 完整性告警与主命令显式开库

## ADDED Requirements

### Requirement: dump 数据链完整性预检

`rtsql dump <db>` SHALL 在导出前对每张用户表执行数据页链完整性预检：自 head 沿页链收集，链终止页与 catalog 持久的尾页一致视为完整。链不完整时 SHALL 向 stderr 输出一行告警（点名表名、catalog 尾页与链终止页、以及「备份可能不完整，库可能已损坏」），SHALL 继续产出可达部分并保持退出码 0。完整库的 dump 行为与输出 SHALL 与修复前逐字节一致（零告警）。

#### Scenario: 干净库 dump 零告警

- **GIVEN** 页链完整的库
- **WHEN** `rtsql dump clean`
- **THEN** stderr 无告警，stdout 输出完整 SQL 文本，exit 0

#### Scenario: 断链库 dump 告警并继续

- **GIVEN** 数据页链在 catalog 尾页之前终止的库（历史缺陷损坏形态）
- **WHEN** `rtsql dump broken 2>err.txt`
- **THEN** stdout 仍输出可达行的 SQL 文本；err.txt 含表名与「可能不完整/已损坏」告警；exit 0

#### Scenario: 告警不进入 stdout 管道

- **GIVEN** 同上断链库
- **WHEN** `rtsql dump broken | grep -c "INSERT INTO"`
- **THEN** stdout 纯 SQL 不含告警文本，管道消费方不受影响

### Requirement: 主命令打开不存在库显式拒绝

主命令（`rtsql <db> <sql>`）在路径解析后、打开前 SHALL 校验库文件存在：不存在（裸名与路径模式同）SHALL 以 `ExitStatus::General`（exit 1）拒绝，错误文案 SHALL 携带库名并指引 `rtsql new <name>`；SHALL NOT 创建任何文件。`new` 子命令（创建通道）SHALL NOT 受此校验影响；其余生命周期子命令既有的缺失拒绝（`resolve_existing_db`）保持不变；lib 层 `Database::open` 对缺失路径的建库行为保持不变。

#### Scenario: 裸名缺失库拒绝

- **GIVEN** `RTSQL_HOME` 下不存在 `typo.db`
- **WHEN** `rtsql typo "SELECT 1"`
- **THEN** exit 1，错误含 `typo` 与 `rtsql new typo` 指引，未创建 `typo.db`

#### Scenario: 路径模式缺失拒绝

- **GIVEN** 路径 `/tmp/nope/x.db` 不存在
- **WHEN** `rtsql /tmp/nope/x.db "SELECT 1"`
- **THEN** exit 1，未创建文件

#### Scenario: new 创建后主命令可用

- **GIVEN** 同上
- **WHEN** `rtsql new typo && rtsql typo "SELECT 1"`
- **THEN** `new` 成功创建，主命令正常执行 exit 0

#### Scenario: lib API 与子命令零回归

- **GIVEN** lib 调用方与既有脚本
- **WHEN** lib `Database::open` 打开缺失路径；`restore`/`import`/`schema`/`dump`/`delete` 操作缺失库
- **THEN** lib 建库行为不变；子命令既有拒绝文案与退出码不变
