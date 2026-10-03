# data-integrity-verification Specification

## Purpose

数据库文件的结构完整性校验面：`rtsql check <db>` 只读全页结构扫描——页 census（0x00/0x01/0x02/0x03/0x04 分类、未知类型点名）、catalog 解析、逐表数据链 walk（防环、terminal 对照 catalog tail、bounds-checked slot 迭代）、溢出链逐链校验（防环、payload 界、total_len 对照）与三类索引树遍历（防环、文件界、页类型、内部节点条目结构）；损坏逐行报告页位置与语义（error 级决定退出码 1，info 级如未引用溢出链与恢复后 tail 良性领先不改退出码），全程不修复不修改库文件，既有打开与生命周期边界（锁/钥/`:memory:`/`--readonly`/pending WAL）沿用。来源：MS27-T02（change `2026-10-02-ms27-overflow-integrity`，方案定稿 design D7「仅 rtsql check」）。

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
