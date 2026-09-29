# RTsql 文件对备份边界实测（.db + .wal 配对行为矩阵）

- Status: active
- Last validated: 2026-09-26
- Environment: Linux x86_64 (WSL2)、Rust debug 构建、RTsql master @ bc1d18d + MS19 Iteration 000 交付（2026-09-26 工作区）；one-shot CLI + `RTSQL_HOME` 隔离
- Source: change `2026-09-26-ms19-cli-admin-hardening` Iteration 000 Act Response › Verification ⑥ 与 Deviations 4/5/7

## 适用范围

验证或复核「拷贝/移动/重命名 RTsql 数据库文件时哪些部分必须成对携带」：

- 备份方案设计或质疑时的实测复核（与 README「备份边界与数据库文件族」声明对照）。
- 存储层改动（checkpoint / WAL / 伴生文件命名 / flush 策略）后回归备份语义。

不适用：dump/restore 逻辑导出（首选备份路径，不经文件对）；Windows 平台（伴生命名与锁语义未测）。

## 前置条件

- 已编译的 `rtsql` 二进制（debug 即可；定时参数随构建速度变化，见失败处理）。
- `RTSQL_HOME` 指向一次性实验目录，与集中存储区隔离。
- 足够写入量使尾部事务可观测：实测用 82 KB / 3000 条 INSERT 脚本。
- 真值基准：完整执行一次不 kill 的同脚本，记录期望行数（实测 938）；干净态基线另记（实测 1 行）。

## 操作步骤

1. 建库建表并干净关闭（close 即 checkpoint，`.wal` 截断至 21 B）——得到「干净态」。
2. 构造「未 checkpoint 尾部态」：spawn `rtsql <db> "<脚本>"`，约 2 s（≈全量耗时 1/3，debug 实测全量 6.2 s）时 `kill -9`。命中执行阶段的标志：`<db>.wal` 显著大于阈值（实测 98406 B）。
3. 准入检查（不满足则回步骤 2 重试）：打开原库 `SELECT COUNT(*)` 等于真值——证明该事务确实活在 WAL 中。
4. **先快照全部副本，再对原库做任何其他操作**——打开即 checkpoint 消费尾部，此后再拷贝测不到丢尾部形态。快照清单：
   - 干净态：仅 `.db`；成对 `.db`+`.wal`；`.db`+`.checkpoint`。
   - 尾部态：仅 `.db`；成对 `.db`+`.wal`；成对移动到另一目录（同基名）；只重命名主文件（`.wal` 留原处）。
5. 副本路径必须含 `/`（绝对路径）——裸名副本被主命令解析进 `$RTSQL_HOME/db/` 并**新建空库**，伪装成「拷贝丢数据」（见 Deviations 7）。
6. 逐形态打开副本，`SELECT COUNT(*)` 与真值比对。

## 验证

成功判据 = 各形态行数与矩阵一致（2026-09-26 实测；938 = 尾部态真值，1 = 干净态基线）：

| 形态 | 实测 | 结论 |
|---|---|---|
| 干净关闭后只拷 `.db` | 1 行 / 1 行 | 完整（close 即 checkpoint，WAL 已截断 21 B） |
| 干净关闭后成对拷 `.db`+`.wal` | 1 行 / 1 行 | 完整 |
| 干净关闭后拷 `.db`+`.checkpoint` | 1 行 / 1 行 | 完整（位点文件非数据必需） |
| 未 checkpoint 态只拷 `.db` | **1 行 / 938** | 静默不完整（打开无报错；被 flush 页携带被杀事务 tx id，按未提交处理） |
| 未 checkpoint 态成对拷 `.db`+`.wal` | 938 行 | 完整（打开时 WAL 重放） |
| 成对移动到另一目录（同名） | 938 行 | 完整（伴生名由主文件基名派生，同基名即可） |
| 只重命名主文件（`.wal` 留原处） | **1 行 / 938** | 尾部丢失，且 98406 B 孤立 `.wal` 被静默忽略 |

原始矩阵与逐形态输出见 Source 的 Verification ⑥；README 双语「备份边界与数据库文件族」节与本表同数据。

## 失败处理

- 脚本 exit 126 且 WAL 零增长：单条 SQL 文本超 Linux `MAX_ARG_STRLEN`（128 KiB）——脚本降到 82 KB / 3000 语句量级（实测 568 KB 直接 exit 126 未执行）。
- kill 后 `.wal` 仍约阈值大小（未命中执行阶段）：调整 kill 时刻；构建变快按全量耗时比例提前（实测 2 s / 6.2 s 命中）。
- 准入检查行数低于真值：被 kill 事务未及落盘——预期内的重试循环，重跑步骤 2。
- 副本「像新建的空库」或行数异常：检查副本路径是否含 `/`——裸名会新建空库而非使用副本。
- 快照后才打开原库导致漏测：无法补救（尾部已被 checkpoint 消费），从步骤 2 重建尾部态。

## 回滚

实验产物均为一次性目录与文件拷贝，删除实验目录即完全回滚；`RTSQL_HOME` 已隔离，不触碰真实数据目录。无不可回滚步骤。

## 证据

- 来源：`openspec/changes/2026-09-26-ms19-cli-admin-hardening/iterations/000-delete-backup-boundary/000-initial.md` › Act Response › Verification ⑥（7 形态矩阵原始数据）、Deviations 4/5/7（flush 半态语义、`MAX_ARG_STRLEN` 与 kill 定时约束、裸名副本陷阱）、Experience Candidates。
- 版本与环境：master @ bc1d18d + MS19 Iteration 000 交付（2026-09-26 工作区，未提交时点）；实验目录 `/tmp/opencode/ms19-i072` 为一次性现场。
- 限制：行数为当次脚本规模（938/1）；更换数据规模后判据按比例重定，形态结论（成对完整、单拷静默不完整、改名丢尾部）不变。
