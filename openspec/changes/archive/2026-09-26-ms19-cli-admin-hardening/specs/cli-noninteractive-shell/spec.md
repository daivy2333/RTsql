## ADDED Requirements

### Requirement: 生命周期子命令 delete

`rtsql delete <db>` SHALL 删除指定数据库的主文件与伴生文件（`<db>.wal`、`<db>.checkpoint`，与引擎 `with_extension` 命名一致）。定位 SHALL 复用既有数据库名称解析（裸名 → `$RTSQL_HOME/db/<name>.db`；含 `/` 形态按路径直开）。删除 SHALL 无交互确认直接执行，并 SHALL 逐文件报告释放结果（路径与释放字节数）；伴生文件不存在时 SHALL 跳过并在报告中说明，SHALL NOT 视为错误。`--dry-run` SHALL 列出将删除的文件与大小且 SHALL NOT 执行任何删除、SHALL NOT 修改文件系统状态。目标库正被打开（advisory 独占锁被其他会话持有）时 SHALL 以 `DatabaseLocked` 显式拒绝（退出码 4）且 SHALL NOT 删除任何文件；锁探测与删除 SHALL 在同一锁持有窗口内完成。库不存在时 SHALL 以既有生命周期命令的缺失语义拒绝（点名路径，退出码 1）。delete SHALL 为文件级管理操作，SHALL NOT 依赖库内容有效性：主文件头部无效或为零字节时 SHALL 同样可删（不初始化、不校验、不要求密钥——加密库的删除与明文库无差别）。

#### Scenario: 删除裸名库并报告释放

- **GIVEN** 集中存储区存在库 `mydb`（主文件与 `.wal` 伴生存在，无 checkpoint 位点）
- **WHEN** `rtsql delete mydb`
- **THEN** 退出码 0，输出逐文件报告主文件与 `mydb.wal` 已删除及释放字节数
- **AND** 文件系统中三个可能路径均不存在；`rtsql list` 不再枚举该库

#### Scenario: dry-run 只列出不删除

- **GIVEN** 同上
- **WHEN** `rtsql delete mydb --dry-run`
- **THEN** 退出码 0，输出与实际删除相同的逐文件清单（含将释放字节数）
- **AND** 所有文件仍在，`rtsql db "SELECT 1"`（指向同库）仍可执行

#### Scenario: 库打开中显式拒绝且零删除

- **GIVEN** 库 `mydb` 已被另一进程打开（持锁中）
- **WHEN** `rtsql delete mydb`
- **THEN** 退出码 4，stderr 点名库被占用（`DatabaseLocked` 语义）
- **AND** 主文件与伴生文件全部原样保留

#### Scenario: 库不存在点名拒绝

- **WHEN** `rtsql delete nosuchdb`
- **THEN** 退出码 1，stderr 点名解析后的路径不存在

#### Scenario: 路径形态与伴生清理

- **GIVEN** 自定义路径 `/tmp/data/x.db` 存在且伴生 `/tmp/data/x.wal`、`/tmp/data/x.checkpoint` 均存在
- **WHEN** `rtsql delete /tmp/data/x.db`
- **THEN** 退出码 0，三个文件全部删除并逐文件报告

#### Scenario: 头部无效文件仍可删

- **GIVEN** 集中存储区存在 `corrupt.db`（内容为随机字节，非有效 RTsql 头）
- **WHEN** `rtsql delete corrupt`
- **THEN** 退出码 0，文件被删除（delete 不做库内容校验，不触发头初始化或校验拒绝）

#### Scenario: 既有生命周期子命令与主命令零回归

- **WHEN** 重复执行既有 `new/list/schema/dump/restore/import` 与主命令用例集
- **THEN** 行为与退出码逐字节一致；裸名与子命令名冲突时子命令优先的既有语义不变（名为 `delete` 的库经主命令形态不可达为既有语义的既知边界）
