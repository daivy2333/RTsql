# sql-attach-session Specification

## Purpose

约束 `ATTACH` 会话语句的附加库注册语义：路径解析复用主库 `resolve_db_path` 语义、会话作用域注册（不写 catalog、无伴生状态）、独占 advisory 锁打开与锁冲突拒绝、注册失败零副作用、别名归一化；重复别名与自附加拒绝面；lib 非会话路径显式拒绝。来源：MS20 一期（change `2026-09-29-ms20-attach-cross-db`，I059 用户方向；DETACH 经用户豁免不在范围并登记 I076）。

## Requirements

### Requirement: ATTACH 语句注册附加库

`ATTACH '<path>' AS <alias>` SHALL 在当前会话注册一个附加数据库文件：路径解析 SHALL 复用 `resolve_db_path` 语义（裸名 → `$RTSQL_HOME/db/<name>.db`、含 `/` 的路径直开）；注册 SHALL 仅为会话作用域——不写入任何 catalog、不产生伴生状态文件、会话（进程）结束时自动释放。附加库 SHALL 以独占 advisory 锁打开（FileStorage `try_lock` 同型），锁冲突 SHALL 以 `DatabaseLocked` 同型错误拒绝（CLI exit 4）。注册失败 SHALL 零副作用（注册表不变）。别名 SHALL 按标识符归一化规则小写处理（`table-name-resolution` 同型）。

> DETACH 注销能力经用户裁定不在本能力范围（2026-09-29 豁免：sqlparser 0.44 无解析面且 one-shot 形态无消费场景；REPL/serve 长会话形态落地时重评，见归档 change proposal 假设 5）。

#### Scenario: 附加库注册成功

- **GIVEN** 存在库文件 `other.db`（含表 `u`）
- **WHEN** one-shot 执行 `ATTACH 'other.db' AS o; SELECT * FROM o.u`
- **THEN** 同一调用内第二条语句可经别名 `o` 读取附加库表

#### Scenario: 裸名路径解析

- **GIVEN** `$RTSQL_HOME/db/other.db` 存在且 `RTSQL_HOME` 已设置
- **WHEN** `ATTACH 'other' AS o`
- **THEN** 按 `$RTSQL_HOME/db/other.db` 打开（与主库裸名解析规则一致）

#### Scenario: 路径不存在拒绝

- **GIVEN** 路径 `missing.db` 不存在
- **WHEN** `ATTACH 'missing.db' AS o`
- **THEN** 点名拒绝且注册表无变化（SHALL NOT 静默创建新库文件）

#### Scenario: 锁冲突拒绝

- **GIVEN** `other.db` 已被另一进程打开（持有独占锁）
- **WHEN** `ATTACH 'other.db' AS o`
- **THEN** 以 `DatabaseLocked` 同型错误拒绝（CLI exit 4），注册表无变化

### Requirement: ATTACH 拒绝面

重复别名 SHALL 点名拒绝且零副作用。attach 主库自身（同 inode，含 symlink/hardlink）SHALL 由文件锁机制拒绝（`DatabaseLocked`，exit 4），SHALL NOT 引入独立路径比较机制。ATTACH 语句 SHALL NOT 进入 plan cache。

#### Scenario: 重复别名拒绝

- **GIVEN** 会话已 `ATTACH 'a.db' AS x`
- **WHEN** 再 `ATTACH 'b.db' AS x`
- **THEN** 点名拒绝（别名已被占用），原注册保持

#### Scenario: 附加主库自身拒绝

- **GIVEN** 主库为 `main.db`（当前进程已持有其独占锁）
- **WHEN** `ATTACH 'main.db' AS m`
- **THEN** 打开附加库时 flock 冲突，以 `DatabaseLocked` 拒绝（exit 4）

### Requirement: lib 非会话路径显式拒绝

经 `Database` 公开执行 API（非 CLI 会话）直接提交 ATTACH 语句 SHALL 显式拒绝并点名 session-only（MS11-T02 事务语句同型先例），且 SHALL NOT 进入 plan cache。

#### Scenario: lib 路径拒绝

- **GIVEN** 经 lib API 打开的 `Database`
- **WHEN** 直接执行 `ATTACH 'a.db' AS x`
- **THEN** 返回错误并点名该语句仅 CLI 会话可用
