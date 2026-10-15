# ISS08 CREATE TABLE 失败残留半表（DDL 无原子性）

- Status: closed
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P1-4）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

列定义页满导致 CREATE TABLE 中途失败时，已写入 catalog 的部分列定义残留为可用「半表」：

- `CREATE TABLE t (id INTEGER, pad TEXT DEFAULT '<5000字符>')` → `Execution error: Page full`（rc 3）
- 失败后 `schema` 输出 `CREATE TABLE "t" ("id" INT PRIMARY KEY);`——表已持久化且只剩首列
- `INSERT INTO t (id) VALUES (1)` 成功——半表可写
- `CREATE TABLE t (id INTEGER)` → `table already exists`——同名表无法重建

机制：CreateTableExecutor 逐条写 catalog 表行/列行，中途失败不回滚已写行（DDL 无原子性）。与文档「拒绝时零副作用」语义矛盾。预期：建表失败零副作用、同名可重试。实际：残留半表，可用且挡住重建。

## 影响

失败 DDL 留下语义不完整的表（列定义缺失），可继续写入导致数据落在不完整 schema 上；同名重建被挡，需 `DROP TABLE` 手工清理。

## 事件记录

- 2026-10-12：外部评测以超大 DEFAULT 触发实测；Explorer 在 HEAD 构建复现全四步（报错/半表可见/可插入/重建被拒）。检测：建表报错后 schema 对照。恢复：`DROP TABLE t` 后重试（评测方未发现此出口）。

## 处置

closed（2026-10-14）——change `2026-10-13-e2e-verification-and-defect-closure` Iteration 002 T3 修复归档。建表 catalog 写入中途失败零残留：内存不注册 / 孤儿列行清理 / `open_or_init` 按 column_count 裁剪 / 页释放，同名可重试；与 ISS14 的计划期预检方向同批收敛（原建议落实）。`tests/ddl_crash_recovery_test.rs` 含失败零残留与可重试见证。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P1-4 段）；复现现场 `/tmp/rt-verify/p4/e.db`。
