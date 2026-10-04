## ADDED Requirements

### Requirement: 建表首数据页初始化

全部建表通道（`CREATE TABLE`、`CREATE TABLE AS SELECT`、import 自动建表、restore、ATTACH 目标库建表）SHALL 在分配表首数据页时将其初始化为合法数据页（page type 0x03 合法页头，零 slot），使新建即含零行表的库满足：`rtsql check` SHALL 以退出码 0 通过且恰输出一行干净确认；`dump` SHALL 无链完整性告警。页初始化 SHALL 持久化，建表后正常 close 与重开两态一致。`check` 对 page type 0x00 页的 error 分级 SHALL 保持不变（损坏检测不放松）；本修复前已存在的 0x00 head 存量表 SHALL 沿最小修复语义——打开路径 SHALL NOT 自愈写入，首次写入或 `TRUNCATE` 后自然转为合法页（写路径既有初始化行为）。

#### Scenario: 新建空库 check 干净

- **GIVEN** `rtsql new` 创建的空库执行裸 `CREATE TABLE`（零行）
- **WHEN** `rtsql check <db>`
- **THEN** exit 0，恰一行 `check ok: ...` 确认，库文件全字节不变（失败边界——修复前：两行 error、exit 1）

#### Scenario: 建表通道全覆盖

- **GIVEN** 分别经 CTAS 空源、import 仅表头自动建表、restore 含空表 dump 得到的零行表（ATTACH 目标库建表在附加库 DDL 计划期拒绝面下不可经 SQL 会话直达，其建表只能经该库自身会话——与前三者收敛于同一 `create_table_with_constraints` 初始化单点，机制级覆盖）
- **WHEN** `rtsql check <db>`
- **THEN** 全部干净通过（同一初始化原语覆盖全部通道）

#### Scenario: 持久两态一致

- **GIVEN** 建表后正常 close 的库
- **WHEN** 重开后 `rtsql check <db>`
- **THEN** 仍干净通过（页初始化持久，恢复重放不破坏）

#### Scenario: 损坏检测不放松

- **GIVEN** 注入 0x00 页的损坏库（既有损坏注入夹具形态）
- **WHEN** `rtsql check <db>`
- **THEN** error 分级与 exit 1 保持（既有 check 损坏注入用例零回归）

#### Scenario: 存量空表最小修复语义

- **GIVEN** lib 直建的 0x00 head 存量夹具表
- **WHEN** 打开（无写入）后 `rtsql check <db>`；随后首次 INSERT 或 `TRUNCATE` 后再次 `rtsql check <db>`
- **THEN** 打开后维持既有 error 报告（不自愈）；首次写入或 TRUNCATE 后转干净
