# data-integrity-verification Specification

## Purpose

数据库文件的结构完整性校验面：`rtsql check <db>` 只读全页结构扫描——页 census（0x00/0x01/0x02/0x03/0x04 分类、未知类型点名）、catalog 解析、逐表数据链 walk（防环、terminal 对照 catalog tail、bounds-checked slot 迭代）、溢出链逐链校验（防环、payload 界、total_len 对照）与三类索引树遍历（防环、文件界、页类型、内部节点条目结构）；损坏逐行报告页位置与语义（error 级决定退出码 1，info 级如未引用溢出链与恢复后 tail 良性领先不改退出码），全程不修复不修改库文件，既有打开与生命周期边界（锁/钥/`:memory:`/`--readonly`/pending WAL）沿用；建表通道初始化首数据页使新建零行表即满足干净校验。来源：MS27-T02（change `2026-10-02-ms27-overflow-integrity`，方案定稿 design D7「仅 rtsql check」）；change `2026-10-03-practical-core-gaps`（建表首数据页初始化）。

## Requirements

### Requirement: 数据页损坏可检出并报告位置

数据页损坏 SHALL 可检出，报告 SHALL 包含页位置与损坏语义，SHOULD NOT 静默返回错误数据。实现方案——页校验和（格式头 flag 演进）、`rtsql check` 全页结构扫描命令、或两者组合——随实现调查定稿并记入 change design；无论采用何种方案，本 Requirement 的可观察行为 SHALL 满足。

#### Scenario: 损坏注入检出

- **GIVEN** 干净库关闭后向某数据页注入字节损坏（如翻转若干字节）
- **WHEN** 触发校验（打开/读取路径或 `rtsql check` 命令，随定稿方案）
- **THEN** 检出损坏并报告页位置与损坏语义，不以正常数据形态静默返回

#### Scenario: 干净库通过

- **GIVEN** 无损坏的正常库
- **WHEN** 触发校验
- **THEN** 校验通过（exit 0 / 无告警），库文件未被修改

### Requirement: 校验面与既有边界协调

完整性校验 SHALL 为只读操作，SHALL NOT 修改库文件内容。`--key`/错钥 exit 5、只读会话、`:memory:`、锁等待预算（exit 4）等既有打开与生命周期边界 SHALL 沿用；既有 WAL 帧 CRC 与恢复行为 SHALL 零变化。校验 SHALL NOT 自动修复损坏数据。

#### Scenario: 校验不修改库文件

- **GIVEN** 任意可校验库（明文或加密）
- **WHEN** 完整执行一次校验
- **THEN** 库文件字节内容不变（伴生文件按既有语义处理）

#### Scenario: 既有边界沿用

- **GIVEN** 加密库、只读会话或被锁占用的库
- **WHEN** 触发校验
- **THEN** `--key`/锁等待/只读边界按既有语义生效，不因校验能力改变

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
