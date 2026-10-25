# Proposal：MS36 历史库与 dump 兼容

## Why

产品已有文件头版本守卫和若干局部向后兼容分支，但「旧库能打开、旧 dump 能恢复」的边界从未被写成一份可测试的合同：README 只描述了「旧 dump 在新库 restore 被拒」这一条负面边界，既没有写清支持哪些旧格式，也没有写清支持到能做什么。后续若发生格式变更（I024），没有基线可以判断新格式是否破坏已有数据（I092）。

本次把「支持的旧库格式与旧 dump 范围」写成正式合同，并用少量固定样本直接断言打开、查询、写入与恢复结果；范围外输入给出明确拒绝与迁移路径。**版本是产品格式契约**，本 change 不引入任何运行身份匹配机制（禁止 Hash/指纹/run-id/manifest/时间顺序证明）。

## What Changes

- 新增规格域 `legacy-format-compat`：定义旧库文件（按目录行布局代际）与旧 dump（按 DDL 渲染代际）的支持范围、逐代能力矩阵、范围外拒绝形态与迁移路径。
- 在 `database-file-format-header` 域增补「格式版本是产品格式契约」：格式版本只有 v1；旧库与新库的差别不在文件头版本，而在目录行布局代际；产品不做旧格式嗅探与迁移。
- 新增兼容样本与断言（`tests/legacy_compat_test.rs` + `tests/fixtures/legacy/`）：旧库文件样本、旧库带未决 WAL 样本、旧 dump 文本样本，断言打开、查询、写入、完整性、打开时重建、恢复结果，以及范围外的明确拒绝。
- 对齐 README 存量兼容段：把范围、拒绝形态与迁移路径写成与合同一致的表述。

**不改变产品代码行为**：本次调查确认支持范围内的行为当前已经成立，范围外的拒绝当前也已显式存在；本 change 的产出是「被断言、被写明」的合同与基线，不是新的兼容实现。新增用例是对既有行为的刻画断言，预期一写即 GREEN。

## 支持范围（结论）

### 旧库文件（按目录行布局代际）

| 代际 | `__tables` 行 | `__columns` 行 | 产生区间 | 打开 | 查询 | 写入 | 完整性 | 打开时重建 | dump |
|---|---|---|---|---|---|---|---|---|---|
| G1（最旧） | 止于 `data_page_tail` | 无 DEFAULT 段 | MS07(2026-08-26) … MS23 前(2026-09-24) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| G2 | 含 unique_roots 段 | 含 DEFAULT 段 | 2026-09-26 … 2026-09-30 前 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| G3（当前） | 含 unique_roots + secondary + kind | 含 DEFAULT 段 | 2026-09-30 至今 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

- 写入语义：被写到的表的目录行**就地升级为当前布局（G3）**；未被写到的表保持原代际。升级后旧库仍可打开、查询、写入。
- G1 是唯一同时缺全部可选尾段的代际，因此**一个 G1 样本即覆盖当前代码的全部向后兼容读分支**（unique_roots 段、secondary 段、kind 字节、列 DEFAULT 段）；G2/G3 由当前原生路径读取，其覆盖面由既有测试套件承担。
- 样本规模保持小（行数 ≪ 300），避开 ISS23 的栈溢出触发面。

### 旧 dump（按 DDL 渲染代际）

- 支持：短类型名 DDL（`INT`/`STRING`/`BOOL`）+ `DATE '…'`/`TIMESTAMP '…'`/`TRUE`/`FALSE`/数值/字符串字面量 → 恢复成功且行数据等价。
- 范围外（明确拒绝，退出码 3，定位到第 k 条/共 n 条）：
  - 非 INTEGER `PRIMARY KEY` → `Plan error: Unsupported constraint: NON-INTEGER PRIMARY KEY`；迁移路径：按当前 dump 的渲染规则去掉 `PRIMARY KEY` 声明后恢复（当前 dump 对非 INTEGER PK 表本就不渲染 PK，往返可达）。
  - 非 INT `UNIQUE` → `Plan error: Unsupported constraint: UNIQUE (INT columns only)`；迁移路径：把被约束列改为 INT 或去掉 `UNIQUE` 声明，重建 schema 后恢复。
  - `NOT NULL` 列含 NULL → `Execution error: NOT NULL constraint violation: column 'x'`；迁移路径：清理或回填该行后恢复。

### 范围外的文件形态

- 文件头 `format_version > 1` → `NewerFileVersion`（既有分类，既有测试覆盖）。
- 无头文件（v1 之前） → `NotADatabase`（既有分类，既有测试覆盖）。

## 范围外

- CI、预编译发布与 crates.io 发布（I051–I053）。
- 键编码格式变更（I024）。
- 伴生文件加密（I060）。
- 产品代码行为变更：`restore` 的逐条 auto-commit 与 fail-fast 语义不变；「多表 dump 中靠后语句被拒时，靠前语句已生效」这一既有契约不在本次改变范围，作为已知边界记录。
- 旧库文件的格式迁移/升级工具：本次只写明范围，不提供迁移实现。
- ISS22–ISS25 的修复：只作约束记录，不在本 change 内处理。

## 授权与批准依据

- 授权：goal 指令「把已经规划的三个 ms 逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」构成 Gate 1 / Gate 2 的用户批准检查点批准；**记为「已批准」，不记为「跳过」**——批准不降低检查标准，BDD 缺口扫描、实现调查、RTM、设计闭合与 Iteration 平衡审计照常完整执行。
- 依据：`.agents/docs/tasks.md` MS36 段（Status `ready`）、`.agents/memory/improvements.md` I092（promoted）。
- 边界：若实施中发现必须超出 MS36 已承诺范围，或出现只有用户能决定的实质未知项，停止并回报，不自行扩大范围。

## 影响面

| 面 | 影响 |
|---|---|
| 产品代码 | **无改动** |
| 新增测试 | `tests/legacy_compat_test.rs`（+1 个测试目标） |
| 新增样本 | `tests/fixtures/legacy/`（旧库文件、旧库未决 WAL、旧 dump 文本 + 来源说明） |
| 文档 | README 存量兼容段 |
| 规格增量 | 新增域 `legacy-format-compat`；`database-file-format-header` 增补一条 requirement |
| 验证 | M19 分批全量回归（全 change 一次，记在最后一个 Iteration 的 Iteration Plan） |
| 全量目标数 | `tests/*.rs` 134 → 135 个文件；目标总数 135 → 136（+ `--lib --bins` 一批） |
