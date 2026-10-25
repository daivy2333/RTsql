## MODIFIED Requirements

### Requirement: 同进程 free-list 复用

被 drop 释放的 page id SHALL 进入 `FileStorage` 的空闲集合；同进程内后续 `allocate_page` SHALL 优先从空闲集合弹出，实现 file_len 单调不增。可分配时机 SHALL 受释放的持久化判定约束：被释放的页只有在其释放已被一次成功完成的 checkpoint 持久化之后才可被分配（详见 `free-page-lifecycle` 域）。释放动作本身 SHALL NOT 写零页；清零发生在该页被弹出分配时。

#### Scenario: drop+recreate 同名表复用 free list

- **GIVEN** 1 个表 `users` 通过 `INSERT 200` 占用 2 个 data page + 1 个 BTree root + 1 个 BTree leaf
- **WHEN** `drop_table("users")` 后完成一次 checkpoint（释放已持久化），再 `CREATE TABLE users (id INT PRIMARY KEY)`
- **THEN** 新表的 `data_page_head` SHALL 等于 `users` drop 时释放的某个 page id（free list 复用）
- **AND** `file_len` SHALL 不超过 `drop 前 + 1`（catalog pages 0/1 + 新表的 1 个 data page；BTree page 也复用 free list）

#### Scenario: 释放未持久化前不复用

- **GIVEN** 1 个表 `users` 已被 `drop_table` 释放，其目录删除尚未被 checkpoint 刷盘
- **WHEN** 同一会话 `CREATE TABLE users (id INT PRIMARY KEY)`
- **THEN** 新表不得占用 `users` 释放的页（该页尚未成为可分配页）
- **AND** `users` 的磁盘页内容保持原样（未被清零）

### Requirement: 跨重启 drop 安全

`drop_table` 后进程重启 SHALL NOT panic 且 SHALL NOT 读 stale BTree 根。释放的 page id 在重启后 SHALL 仍不被任何持久对象引用（catalog 行已删），因此可以安全地被重新分配：重启时 SHALL 依据持久目录重建空闲页集合，把持久目录引用不到的页纳入本次打开的可分配页集合（详见 `free-page-lifecycle` 域）。重建 SHALL NOT 把持久目录仍登记的页纳入可分配页集合。

#### Scenario: restart after drop 不 panic

- **GIVEN** 1 个表 `users` 已被 drop（catalog 行已抹）
- **WHEN** 进程重启 + `Database::open(path)`
- **THEN** SHALL NOT panic
- **AND** `get_table("users")` SHALL 返回 `Err(TableNotFound)`

#### Scenario: restart after drop+recreate 正常工作

- **GIVEN** 1 个表 `users` 已被 drop
- **WHEN** 进程重启 + `Database::open(path)` + `CREATE TABLE users` + `INSERT` + `SELECT`
- **THEN** 全部操作 SHALL 成功
- **AND** `users` 的 BTree SHALL 从新分配的 page 开始，无 stale root

#### Scenario: restart after drop 复用释放的页

- **GIVEN** 1 个表 `users` 已被 drop，且该 drop 已完成一次 checkpoint（目录删除已持久化）
- **WHEN** 进程重启 + `Database::open(path)` + `CREATE TABLE users`
- **THEN** 新表占用的是 `users` 释放的页号，库文件体积不因新建表而增长
- **AND** 库中其他活表的页不进入可分配页集合
