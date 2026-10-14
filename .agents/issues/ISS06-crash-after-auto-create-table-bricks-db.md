# ISS06 import 自动建表后未 checkpoint 崩溃 → 整库砖化（恢复路径不可达）

- Status: open
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P0-2）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

建表 DDL 的 catalog 脏页仅存在于缓冲池、WAL 已含该表已提交行记录时进程崩溃（SIGKILL），之后整库所有打开路径永久失败：

- 读写打开：exit 1 `WAL redo failed: table '<t>' referenced by committed WAL records is missing from the catalog; … cannot be opened and no records were skipped`
- `check`：exit 1 `database has pending WAL, open read-write once to recover` ——**提示的恢复路径（读写打开一次）正是失败的那个操作，死锁闭环**。
- `schema`/`checkpoint` 等全部 exit 1。唯一出路是删库。

机制（各环节 Confirmed）：DDL 不写 WAL（MS18 设计事实）；catalog 页仅在 checkpoint/close/缓冲池驱逐时落盘；恢复重放对 catalog 缺表硬失败不跳过（K05；四要素定向文案为 practical-core-gaps T11 引入，把形态说清但未改变不可达结局）。触发窗口：自动建表 import（表首次出现即崩溃）+ 数据量小（catalog 页不被驱逐刷盘）+ 未达 16MiB 自动 checkpoint 水位。对照：表已存在的库同型崩溃正常恢复；200000 行大数据量触发池驱逐刷盘则不砖化。

预期：已提交数据在崩溃后可恢复打开（WAL 重放能容忍「DDL 落盘落后于数据 WAL」的窗口，或提供可达的恢复出口）。实际：库永久不可打开。

## 影响

可用性红线：崩溃窗口内已提交的全部数据不可达，文档给出的恢复路径在该场景不可达，唯一出路 `rtsql delete` 删库。外部评测实际命中（perfdb 砖化样本留存 `/tmp/rtsql-eval/db/`）。

## 事件记录

- 2026-10-12：外部评测 import 自动建表后 SIGKILL 实测砖化；Explorer 全新因果复现（10000 行 import 5 秒后 kill → WAL 279KB 未达水位 → 全命令 exit 1）并在评测现场 perfdb 副本上复现同一行为。检测：崩溃后任意命令。恢复：无（删库唯一出路）；`RTSQL_WAL_CHECKPOINT_BYTES` 调小可缩小窗口（规避非修复）。

## 处置

待用户裁定立项（涉及设计决策：redo 容忍缺表延迟建表 / DDL 入 WAL / 打开时先刷 catalog 脏页等方案取舍；`check` 的恢复指引文案需与最终方案同步）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P0-2 段：全新复现步骤、窗口边界对照、评测现场双验证）；复现现场 `/tmp/rt-verify/brick3/`、`/tmp/rtsql-eval/db/perfdb.*`。
