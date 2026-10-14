# ISS08 CREATE TABLE 失败残留半表（DDL 无原子性）

- Status: open
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

待用户裁定立项（与 ISS12 同场景交叉：错误文案点名 DEFAULT 过大不改变残留问题，两缺陷独立验收；修复方向候选——预检 DEFAULT 页内容量先行拒绝（计划期、零副作用）或 catalog 写入失败回滚）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P1-4 段）；复现现场 `/tmp/rt-verify/p4/e.db`。
