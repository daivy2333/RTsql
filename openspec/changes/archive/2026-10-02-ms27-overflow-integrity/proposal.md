# MS27 存储演进：超页元组溢出页链 + 数据完整性校验

## Why

路线 `.agents/docs/tasks.md` 的 MS27（存储演进批）尚未实施，含两个无硬依赖的 task：

- **T01 溢出页链**：超过单页容量（4096B）的元组在 `src/storage/page_format/slotted_page.rs:234-235` 直接失败 `"No enough space in page"`，且 `Slot` 为 6B 定长（offset/length 各 u16），行值结构性受限于单页。agent 存长 JSON/文档为高频场景（I069 只覆盖 BLOB/DECIMAL 类型面，不含此存储面）。
- **T02 完整性校验**：WAL 帧有 CRC 但数据页无任何校验，亦无完整性校验命令——与 I072 备份边界同主题。

## What Changes

用户于 2026-10-02 批准两项集中决策（见「已批准的集中决策」）。本文件为需求基线草稿，实现调查完成前 design/tasks/Gate 2 均为草稿。

- R1：超过单页容量的行值（主场景 String 长文本/JSON）SHALL 经溢出页链存取——写入读回逐字节一致，写路径覆盖既有全部行写入通道（INSERT VALUES/子集+DEFAULT、UPDATE 增长与缩回、UPSERT/REPLACE、CTAS 物化、INSERT SELECT、import/restore/dump）。SQL 层 SHALL 对溢出透明，不设人为硬上限（受链长字段与页号空间自然约束）；e2e 验收规模基线为 ~1MB 单元组（≈250 溢出页）。
- R2：溢出页 SHALL 按既有加密页机制联动（4124B 步长、页级加密）；`:memory:` 后端与只读会话 SHALL 同等支持超页行读写。
- R3：含超页行的写入 SHALL 在干净重开与 WAL 崩溃恢复两态下数据与 PK/UNIQUE/二级索引一致；溢出页重放与既有按 logical_id 强制落位语义协调。
- R4：溢出页链 SHALL NOT 产生孤儿页。释放点：版本 slot 被 GC 删除时链一并释放、drop_table/TRUNCATE 随数据页释放、写入语句溢出链分配中途失败时清理本次已分配页（对齐 MS07-T02 drop 页释放先例）；回滚沿用既有语义保留 aborted slot，其链保持可达非泄漏。
- R5：既有无溢出页库文件 SHALL 零迁移打开读写；非超页行为（页内布局、B-Tree 32B 定长键、既有错误面）SHALL 零回归。
- R6：数据页损坏 SHALL 可检出并报告位置（页号与损坏语义）。实现方案——页校验和（格式头 flag 演进）、`rtsql check` 全页结构扫描命令、或两者组合——随实现调查定稿，选择与理由记入 design。
- R7：完整性校验 SHALL 为只读操作（不修改库文件）；既有边界沿用——`--key`/只读会话/`:memory:`/锁等待、WAL 帧 CRC 行为不变。

## 场景草图

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 超页写读 | String 列表，页 4096B | INSERT ~100KB 行值 | 写入成功；SELECT 读回逐字节一致；重开一致 |
| S2 页边界 | 页内最大可容纳行 / 恰超 1 字节行 | 分别写入 | 两者均成功读回一致；页内可容纳行的既有存储形态不变 |
| S3 增长与缩回 | 页内行与超页行并存 | UPDATE 增长为超页 / 超页行缩回页内 | 均成功且读回一致；旧版本溢出链可达，随旧版本 slot 生命周期（GC 删除时释放） |
| S4 全写通道 | 目标表有约束/索引/DEFAULT | UPSERT/REPLACE、CTAS、INSERT SELECT、import、restore 写超页行 | 与单页行同语义；约束/索引/计数一致 |
| S5 恢复两态 | 超页行写入后 WAL 未 checkpoint | 崩溃重开恢复 / 干净重开 | 两态行数据与 PK/UNIQUE/二级索引一致；恢复幂等 |
| S6 生命周期释放 | 表含超页行 | GC 回收旧版本、写入中途失败、drop_table、TRUNCATE | 对应溢出页物理释放；回滚沿用既有语义保留 slot（aborted），链保持可达非泄漏；无孤儿页 |
| S7 旧库兼容 | 既有无溢出页库文件 | 打开读写 | 零迁移；既有行为与文件布局不变 |
| S8 运行形态 | `:memory:` 库 / 只读会话 | 超页写读（memory）/ 读超页行（只读） | 同等可用；只读零写入保持 |
| S9 加密联动 | 加密库（对钥） | 超页写读 + 崩溃恢复 | 往返一致；错钥/无钥既有拒绝面保持 |
| S10 既有错误面 | 非 String 超大值路径（如超长日期字面量） | 写入/解析 | 既有解析与类型拒绝面保持，不因溢出能力放宽 |
| S11 规模基线 | 空 String 列表 | INSERT ~1MB 行值并读回（含恢复） | 成功且一致；作为验收规模锚点 |
| S12 多版本可见性 | 超页行多版本（UPDATE 后） | RR/RC 快照读旧版本 | 既有 MVCC 语义保持；旧版本溢出链随 GC 释放 |
| S13 损坏检出 | 干净库注入数据页字节损坏 | 触发校验（随方案：打开/读取或 `rtsql check`） | 检出并报告页位置与损坏语义；不静默返回错误数据 |
| S14 干净库通过 | 无损坏库 | 同上触发校验 | 通过（exit 0 / 无告警），库文件未被修改 |
| S15 校验边界 | 加密库/只读会话/锁占用库 | 触发校验 | `--key`/锁等待/只读边界沿用既有语义 |
| S16 WAL CRC 保持 | 正常写入流程 | 观察 WAL 行为 | 既有 WAL 帧 CRC 与恢复行为零变化 |

## 已批准的集中决策

1. 2026-10-02 用户裁定 **T01 + T02 合一个 change**（选项「T01 + T02 合一个 change」），覆盖 tasks.md 对 MS27「2 个中 change 各自独立验收」的注记；Iteration 划分须保证两项各自独立可验收、诊断边界不交叉。
2. 2026-10-02 用户裁定超页元组 **e2e 验收规模基线 ~1MB**（选项「~1MB（推荐）」）；实现不设人为硬上限。
3. 2026-10-02 用户指令将 Iteration 000 Review 报告的 catalog DEFAULT 段长字符串回绕（`serialize_catalog_column_row` String 臂 `as u16`）**并入当前 Iteration 001 重新规划、一并完成**（原话：「把这个issue直接放到当前iter的工作里面，重新规划，一起完成了吧」）；不登记 Issue。调查确认生产不可达（列行页内容量 ~4074B 先行页满拒绝），定位为表示一致性硬化 + 边界文档化，见 design D11 与 tasks.md T12。

## Non-goals

BLOB/DECIMAL 类型（I069，溢出页是其前置但类型面独立）；压缩；在线重整/VACUUM 全库搬家；校验后自动修复（只检出报告不修复）；catalog 系统表行溢出（列定义行保持页内，若调查发现可达超页则记录边界不扩围）；跨文件/附加库校验编排。

## 工作状态与边界

- Phase 1 CLARIFY：completed；场景扫描、需求基线、集中决策完成。
- Gate 1：PASS；2026-10-02 用户批准（选项「批准（推荐）」）。
- Phase 2 INVESTIGATE AND PLAN：completed；Current-State Evidence 与关键决策 D1–D10 见 design（含 R4 生命周期措辞按调查事实校准——回滚沿用既有 aborted slot 语义）；任务 T1–T11、三个 Iteration、RTM 见 tasks.md；当前执行契约 `iterations/000-initial/000-initial.md`（draft，待 Gate 2）。
- T02 方案随调查定稿为 `rtsql check` 结构扫描命令（design D7），2026-10-02 随完整计划获批（选项「仅 rtsql check（推荐）」）。
- Gate 2：PASS；2026-10-02 用户批准完整计划（选项「批准完整计划」）。当前 Cycle Status=ready；尚无 Act 实施授权。
- Persisted Evidence：none；不创建 Evidence。
- SKIPPED：产品代码修改、全局状态同步、实施、归档，均不属于本次 Plan 授权。

## Impact 与回退

关联路线 MS27-T01/T02；来源缺陷依据见 tasks.md MS27 Scope 表（`slotted_page.rs:212` 超页失败〔现行 :234-235〕、数据页无 checksum/无 check 命令）。

影响面：`src/storage/page_format/`（SlottedPage/Slot/tuple 序列化）、数据页分配与 free-list、`src/storage/data/table_manager.rs`（写路径/释放路径）、`src/storage/file_storage.rs` 与 `crypto.rs`（页步长/加密）、`src/wal/recovery.rs`（重放）、`src/storage/file_header.rs`（若采校验和 flag）、`src/cli/`（若采 `rtsql check` 子命令）、GC/回滚路径、两后端测试与全量回归。

回退：本变更含磁盘格式演进（溢出页；若采校验和另有格式头 flag）。回退代码后，新写入的超页行库文件对旧二进制的可读性不承诺（向前兼容不在目标内）；既有无溢出页数据不受影响。已提交数据的既有单库保证不变。
