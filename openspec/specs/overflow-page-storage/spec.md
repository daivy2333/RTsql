# overflow-page-storage Specification

## Purpose

超页元组的存储格式演进：超过单页内联容量（slot_data > 4074B）的行值经主页 slot 内 locator（`[0xFF][total_len u32][首溢出页 u32]`，VersionHeader 恒内联）与溢出页链（page_type 0x04、chunk ≤4080B/页）透明存取，SQL 层零感知、无人为长度上限（长字符串增量 tag 0x08 消除 u16 长度前缀回绕，catalog DEFAULT 段同语义）；全写通道、加密 4124B 步长、`:memory:`/只读、WAL 恢复重放（链页重分配 + locator 末步落位幂等）与生命周期释放（GC/drop/TRUNCATE/写入失败清理）联动，既有单页行为与旧库文件零迁移零回归。来源：MS27-T01（change `2026-10-02-ms27-overflow-integrity`）。

## Requirements

### Requirement: 超页元组经溢出页链存取

超过单页容量的行值 SHALL 经主页 slot 与溢出页链存储，读回值与写入值逐字节一致。溢出对 SQL 层 SHALL 透明，系统 SHALL NOT 对行值设置小于自然约束（链长字段位宽与页号空间）的硬上限。既有全部行写入通道——INSERT VALUES（含子集列与 DEFAULT）、UPDATE（含增长为超页与缩回页内）、UPSERT/REPLACE、CREATE TABLE AS SELECT 物化、INSERT SELECT、import 与 restore——SHALL 同等支持超页行。列声明 DEFAULT 字面量的 catalog 持久化段 SHALL 与 tuple 值表示同语义：超过 65535B 的 String DEFAULT 段 SHALL 使用长形式（tag 0x08 + u32 长度前缀），等于或小于 65535B 的段 SHALL 保持既有字节布局；DEFAULT 字面量长度受 catalog 列行页内容量的既有边界约束，超限建表按既有错误面拒绝。

#### Scenario: 超页行写入读回一致

- **GIVEN** 目标表含 String 列，页容量 4096B
- **WHEN** 写入约 100KB 的 String 行值并 SELECT 读回
- **THEN** 读回值与写入值逐字节一致，干净重开后仍一致

#### Scenario: DEFAULT 段长字符串表示一致

- **GIVEN** 列声明携带超过 65535B 的 String DEFAULT 字面量
- **WHEN** catalog 列行 DEFAULT 段序列化并读回（段级往返）
- **THEN** 段使用 tag 0x08 + u32 长度前缀，读回字面量与声明一致；等于或小于 65535B 的 DEFAULT 段字节布局与既有格式逐字节不变

#### Scenario: 页边界两侧行均可达

- **GIVEN** 目标表含 String 列
- **WHEN** 分别写入页内最大可容纳行与恰超页容量 1 字节的行
- **THEN** 两者均写入成功且读回一致；页内可容纳行不启用溢出链，既有存储形态不变

#### Scenario: 全写通道同语义

- **GIVEN** 目标表含 String 列并声明约束/索引/DEFAULT
- **WHEN** 经 UPSERT/REPLACE、CTAS、INSERT SELECT、import、restore 写入超页行
- **THEN** 与单页行语义一致：约束强制、索引条目、affected 计数、往返保真均保持

#### Scenario: 规模基线

- **GIVEN** 空 String 列表
- **WHEN** 写入约 1MB 的行值并读回（含崩溃恢复路径）
- **THEN** 成功且逐字节一致

### Requirement: 运行形态与加密联动

溢出页 SHALL 按既有页级加密机制处理（加密库 4124B 页步长、页级加解密），`:memory:` 后端与只读会话 SHALL 同等支持超页行读写。

#### Scenario: 加密库超页往返

- **GIVEN** 以 `--key` 创建的加密库
- **WHEN** 写入超页行、读回并执行崩溃恢复
- **THEN** 明文语义一致；错钥/无钥既有拒绝面保持

#### Scenario: 内存库与只读会话

- **GIVEN** `:memory:` 库写超页行；同一文件库以只读会话打开
- **WHEN** 内存库读写超页行；只读会话 SELECT 超页行
- **THEN** 内存库同等可用且进程退出即弃；只读会话零写入保持

### Requirement: 恢复两态一致

含超页行的写入 SHALL 在干净重开与 WAL 崩溃恢复两态下产生一致的数据与 PK/UNIQUE/二级索引。溢出页重放 SHALL 与既有按 WAL 记录 logical_id 强制落位的收敛语义协调；恢复重跑 SHALL 幂等。

#### Scenario: 超页行崩溃恢复

- **GIVEN** 超页行写入后 WAL 未 checkpoint
- **WHEN** 崩溃重开执行恢复
- **THEN** 恢复成功无 RedoFailed，行数据与三类索引与运行期提交终态一致；重复恢复幂等

### Requirement: 溢出页物理生命周期

溢出页链 SHALL NOT 产生孤儿页（不被任何 slot 引用且未被释放的页）。释放点：版本 slot 被 GC 删除时链一并释放；drop_table、TRUNCATE 时随数据页释放；写入语句在溢出链分配中途失败时释放本次已分配页。回滚 SHALL 沿用既有语义保留 slot（aborted 标记），其溢出链保持可达、随 slot 存续，SHALL NOT 视为泄漏。

#### Scenario: 表级释放

- **GIVEN** 表含超页行
- **WHEN** 执行 drop_table 或 TRUNCATE
- **THEN** 数据页与溢出页均物理释放，重开后空间状态一致

#### Scenario: 版本级释放

- **GIVEN** 超页行经 UPDATE 产生旧版本（缩回页内）后
- **WHEN** 旧版本 slot 被 GC 回收（gc_table）
- **THEN** 旧版本溢出链释放，无孤儿页

#### Scenario: 写入失败不泄漏

- **GIVEN** 溢出链写入中途失败（如 IO 错误）
- **WHEN** 语句以错误收场
- **THEN** 本次已分配的链页被释放，不残留孤儿页；locator 未落位则无 dangling 引用

### Requirement: 既有行为与格式兼容

既有无溢出页库文件 SHALL 零迁移打开读写。非超页行的页内布局、B-Tree 32B 定长键面、既有解析与类型拒绝面（如超长日期字面量）SHALL 零回归。

#### Scenario: 旧库零迁移

- **GIVEN** 本变更前创建的库文件
- **WHEN** 以新二进制打开并读写
- **THEN** 直接可用，无需迁移，既有数据与行为不变

#### Scenario: 非超页行为零回归

- **WHEN** 执行既有全量测试面
- **THEN** 单页行行为、MVCC 可见性、错误面全部保持
