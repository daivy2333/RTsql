## MODIFIED Requirements

### Requirement: DDL 语句边界 catalog 有序落盘

DDL 语句（CREATE TABLE、CREATE INDEX、CREATE VIEW、ALTER TABLE ADD COLUMN，含 import 自动建表与 CTAS 建表段）执行成功返回前，其 catalog 效果 SHALL 有序落盘：被引用页（数据页头、索引根页、`__columns` 列行页）先落盘，`__tables` 行（或其更新副本所在页）最后落盘（commit point）。此后未 checkpoint 崩溃（SIGKILL），读写打开 SHALL 恢复可达——WAL 重放能找到表定义，已提交数据完好，`check`/`schema`/`checkpoint` 正常（`check` 的「open read-write once to recover」指引在该形态下可达）。恢复重放的硬失败语义（K05：catalog 缺表即拒绝打开、不跳过记录）SHALL 保持不变；建表前/建表冲刷中段崩溃的既有语义（表定义未持久 = DDL 视同未发生）SHALL 不变。`DROP TABLE`、`DROP INDEX`、`DROP VIEW` 和 `TRUNCATE` SHALL 保持 checkpoint 边界，不在语句成功返回前把 catalog 删除当成 commit point。`ALTER TABLE DROP COLUMN` 成功返回后的崩溃结果由 `drop-persistence-boundary` 规定，不在本要求中改成或撤销语句内的 catalog 刷盘。

#### Scenario: import 自动建表后未 checkpoint 崩溃可恢复

- **GIVEN** `rtsql new` + `import --csv`（自动建表 + 已提交行、WAL 未达自动水位、无 checkpoint）+ SIGKILL
- **WHEN** 读写打开（或 `check`/`schema`/`checkpoint`）
- **THEN** 打开成功、表定义与已提交数据完好、`check` exit 0 干净（失败边界——修复前：全命令 exit 1 整库砖化，`check` 提示的恢复路径即失败操作，死锁闭环）

#### Scenario: DDL 各形态建后即崩恢复

- **GIVEN** 分别执行 CREATE TABLE（含约束/DEFAULT）、CREATE INDEX（含回填）、CREATE VIEW、ALTER ADD COLUMN、CTAS、import 自动建表后立即 SIGKILL（无 checkpoint/close）
- **WHEN** 读写重开
- **THEN** 表/索引/视图/列定义全部生效，数据可达，`check` 干净

#### Scenario: 小数据量不砖化（池驱逐不可依赖）

- **GIVEN** 小数据量（catalog 页不被缓冲池驱逐刷盘）+ WAL 已有该表已提交行 + 未 checkpoint + SIGKILL
- **WHEN** 任意开库命令
- **THEN** 恢复可达（失败边界——修复前：依赖池驱逐偶然刷盘才不砖化；确定性砖化窗口）

#### Scenario: 既有恢复与零工作语义零回归

- **GIVEN** 表已存在的库同型崩溃；干净库（WAL 从无事务记录）纯读会话
- **WHEN** 恢复打开 / close / 显式 checkpoint
- **THEN** 既有恢复行为与 MS18 零工作跳过语义逐字节不变
