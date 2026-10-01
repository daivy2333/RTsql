# catalog-durability-and-cli-surface — Design

## 现状与目标行为总览

| 面 | 现状 | 目标 |
|---|---|---|
| catalog 定长字段更新 | 「追加新行副本 + 删旧行」；页满静默 `Ok(())`；每次更新永久泄漏行宽字节（`__tables` 页 ~100 次更新窒息） | 等长更新原地覆写活行 slot（零泄漏、零追加）；页满/失败一律 `Err` 点名 |
| catalog 变长行更新 | 同一 append+delete 路径（CREATE INDEX 增段），页满静默丢失 | 跨页链追加新副本 + 旧副本清理 + `scan_tables` 同名去重 |
| 跨进程写 durability | 窒息后每进程读陈旧 tail，孤儿页段累积，扫描/dump 永久致盲 | tail 等字段跨进程持久化；多轮 open→写→close count 单调 |
| 表别名 | FROM 丢弃别名；限定名四消费点只认真实表名；单表静默丢弃限定符 | 别名注册 + 单点归一化；全子句面限定引用可用；未知限定符点名 |
| ORDER BY 限定键 | `extract_column_name` 仅收裸 Identifier，两段一律 ParseError | 两段键经别名归一化解析，与裸键/输出别名同解析面 |
| dump | 沿链扫描，链断静默截断 | 前置链完整性预检，断链 stderr 告警 + rc 0 |
| 主命令开库 | 缺失库静默建 0 字节新库 | 显式拒绝 exit 1 + `rtsql new` 指引；lib API 不变 |
| 锁冲突拒绝时序 | `WouldBlock` 立即拒绝——平台 flock 释放可见性窗口下，同进程 close→立即重开偶发假 `DatabaseLocked`（WSL2 6.18.33.2，纯 std 探针 2/20000） | 冲突后有界退避重试（总窗口 ~8ms）再拒绝；拒绝语义与错误面不变 |
| SKILL.md | 功能目录未成命令行清单 | 命令行形式功能目录，与实现逐条一致 |

## 关键决策

### D1 等长原地更新（Iteration 000 核心选择）

`SlottedPage` 新增 `update_slot_in_place(logical_id, data)`：按 logical_id 定位 slot，校验 `slot.length == data.len()` 后覆写数据区字节，**不修改 header**（slot_count/free_space_offset/next_logical_id 不变，不触发 `serialize` 回写）。

理由与后果：
- catalog 现有全部更新调用方均为定宽字段值变化（tail/root 均 u32、column_count u32、ordinal root u32），序列化行长恒等——原地写回适用面即 100% 热路径。
- 同时消除缺陷 A（泄漏源头）与缺陷 B（页满静默路径在等长分支不复存在）。
- **对既有窒息页自愈**：活行仍在原 slot，原地写回不依赖页剩余空间——用户现存损坏库的 tail 更新即刻恢复持久化，无需迁移。
- 原子性继承页锁（`modify_page` 临界区），无追加/删除两步崩溃窗口。

### D2 变长更新重定位（覆盖 CREATE INDEX 增段等未来/低频路径）

长度变化的更新走：`append_to_chain`（catalog.rs:583 既有跨页链追加原语，`insert_column_row` 同型消费）写入新副本 → 链上删除同名旧副本（含跨页）。`scan_tables` 增同名去重（保留 logical_id 最大者），把 D2 的两步崩溃窗口收敛为确定性语义。头页指针维护跟随 `append_to_chain` 既有语义（`TABLES_PAGE_ID` 为常量头，链上新增页经尾页 next 链接）。

### D3 静默路径诚实化

`update_field_in_chain` 删除 `Err(_) => return Ok(())` 分支（catalog.rs:793-796）；原地分支的等长校验失败、变长分支的追加/删除失败一律 `Err`（携带表名与原因）向上传播。调用方错误语义逐一复核：`write_tuple` tail 更新失败 → INSERT 以 StorageError 失败（零副作用，磁盘/IO 错误本应失败）；truncate/ALTER/index split 写回失败 → 传播点名；recovery 路径写回失败 → 恢复显式 `Err`（K05 既有纪律）。B-tree 分裂写回位于 `index_manager.rs:126-135` 的 catalog 上下文调用，失败传播不得吞错（await 点形态属局部实现细节，Act 按传播语义落地）。

### D4 别名注册模型（Iteration 001 核心选择）

`PlanBuilder` 增独立映射 `table_aliases: HashMap<String, String>`（别名 → `self.tables` 真实键），归一化函数 `resolve_table_qualifier(&str) -> Option<&str>` 单点实现：先查别名映射，再原样返回（供 `self.tables` 直接命中，含 attach 复合键）。四个限定名消费点统一消费：

1. `resolve_qualified_column` / `resolve_column_ref`（expression.rs:81，Hash 臂 ON 腿/WHERE/聚合参数通道）；
2. `qualified_column_expression`（expression.rs:249，无布局臂与 NLJ 布局臂——布局表名构造处同样先归一化）；
3. JOIN 输出列过滤与 `resolve_join_projection_indices` 的 qualifier 匹配（query.rs:498-541 / 2733-2781，`OutputColumn.table` 保持真实名，qualifier 先归一化再比较）；
4. ORDER BY 键解析（query.rs:2901 + 2059-2086 校验集）。

注册点：FROM 基表臂（query.rs:334）、JOIN 右臂（query.rs:426）、CTE/视图 effective 名臂、attach 复合键臂（复合键本身已是 `self.tables` 键，无需别名条目）。`self.tables` 键空间不污染（避免别名与表名/复合键冲突语义）。别名→多表（自连接同表两别名）天然支持（两键各自映射）。

未知限定符错误形态：`TableNotFound(qualifier)`（42P01，语义 = FROM 中无该关系）——修复后该错误仅在限定符确实不存在时出现，现网「别名被当表名」的误导错误随之消失。

### D5 限定形态边界统一

- 两段 `q.c`：`q` 经 D4 归一化（别名或真实表名/复合键）后按既有通道解析。
- 三段 `a.b.c`：仅当 `a.b` 命中 `self.tables` 键（attach 复合键）可达；本地下（如 `demo.people.name`）从「单表静默丢弃限定符意外成功 / JOIN 失败」统一为计划期点名拒绝。
- 单表不再静默丢弃限定符（`extract_columns` 对 CompoundIdentifier 取末段的行为保留于**输出列名/表头**职责，但**解析职责**改走归一化通道；`p.name` 在已声明别名 p 时经 D4 命中，未声明的 x.name 拒绝）。

### D6 ORDER BY 限定键

`extract_column_name` 扩展两段形态（返回限定符 + 列名）；键解析序维持既有语义（输出别名优先 → 可见列），限定键先归一化再进同一解析面；Sort 装配（decorate-sort-undecorate / `OrderByColumn`）解析到输入列索引，表头与行形状不变。`order-by-alias-sorting` R3 的「`ORDER BY t.col` 维持 ParseError」边界注记随本 change 终止。

### D7 dump 链完整性预检

`TableManager` 新增只读 helper（复用 `collect_data_pages` 链遍历形态）：自 head 沿 next 收集，链终止页 ≠ catalog tail ⇒ 不完整。`dump_db` 在打开后、导出前调用；不完整 ⇒ stderr 一行告警（点名表名、catalog tail 与链终止页），继续导出，rc 0。干净库零告警零行为变化。

### D8 主命令显式开库

`execute_command` 主命令臂在 `resolve_db_path` 之后、打开之前校验存在性；缺失 → `ExitStatus::General`（exit 1），文案 `database '<name>' does not exist; create it first with: rtsql new <name>`。`new` 子命令不校验（创建通道）；其余生命周期子命令既有 `resolve_existing_db` 拒绝保持；lib `Database::open` 建库语义不变。

### D9 文档面（SKILL.md 重构）

`rtsql-docs/SKILL.md` 增「命令行功能目录」章节：主命令（`rtsql <db> <sql>` 全 flag：`--format`/`--key`/`--readonly`/`:memory:`/多语句分片）、10 个生命周期子命令逐条一行示例、env（`RTSQL_KEY`/`RTSQL_HOME`/`RTSQL_WAL_CHECKPOINT_BYTES`）、退出码表（0/1/2/3/4/5/130/143）、本 change 新语义（缺库拒绝/dump 告警/别名支持）。真值来源 = `src/cli/mod.rs` `Command` 枚举 + 实现 + 本 change specs；README 双语对应段核验一致。

### D10 锁冲突有界退避重试（Iteration 001 replan 扩入，2026-10-01 用户裁定）

**根因（Plan 调查 2026-10-01，证据链完整）**：进程内 `close(fd)`（含 `Database` drop）到下一次对同文件 `flock` 之间，内核的锁释放存在负载相关的短暂可见性窗口——`File::try_lock` 偶发 `WouldBlock`，表现为假 `DatabaseLocked`。证据：① 纯 std 探针（零 rtsql 代码）：同进程 open→flock(EX)→close→立即 reopen→flock(EX)，20000 轮 × 8 路并发负载下 2 次 EAGAIN（WSL2 6.18.33.2-microsoft-standard）；② `readonly_session_test` 默认并行度 6 轮 2 轮失败、失败集逐轮漂移、`--test-threads=1` 全绿；③ strace `-f -yy` 全进程树取证：失败路径仅测试进程自身触碰，前两个 fd 的 `close()=0` 均有踪，第三次 open 的 `flock(LOCK_EX)` 仍 EAGAIN——内核态无在册持有者。产品影响面：lib 消费形态「close/drop 后立即重开同一路径」（嵌入式宿主、测试）；one-shot CLI 跨进程先后执行不受影响（进程退出释放是确定性的）。

**设计**：`FileStorage::open_with_key` 与 `open_readonly_with_key` 的 `WouldBlock` 分支改为**同 fd 有界重试**（`try_lock` 失败不消费 fd）：间隔 2ms 重试，连同首次共 5 次尝试（总窗口 ~8ms），窗口耗尽仍 `WouldBlock` → 既有 `DatabaseLocked`（携带路径）不变。理由与后果：

- 拒绝语义不变、错误面不变：真冲突（活跃持有者）在窗口耗尽后照旧拒绝，锁矩阵测试与 CLI exit 4 仅增加 ≤ ~8ms 时延。
- 与 SQLite `busy_timeout`（推荐非零默认）同族；单写者模型与 `database-file-lock` 的 advisory 语义不动。
- 默认启用、零新 API、零 env 通道：窗口常数不构成配置面；最小变更面。若未来需要可调窗口，另立 improvements。
- 重试睡眠为同步 `std::thread::sleep`：open 路径本就在调用方线程执行同步 open/头校验，不引入新的异步语义。

## 责任边界

- **修改**：上述九面所列符号（含 D10 的 `src/storage/file_storage.rs` 锁分支与 `tests/database_file_lock_test.rs`）。
- **保持**：页格式（4096B/slot 布局/header 16B）不变；lib `Database::open` 建库行为不变；attach 两段表引用 attach-first 语义不变；`OutputColumn.table` 真实名语义不变（归一化在消费侧）；WAL/checkpoint/恢复链路不变（P0 纯存储结构层修复，经 close checkpoint 落盘验证）；锁的 advisory 语义、单写者模型与 `DatabaseLocked` 错误面不变（D10 仅增加有界重试时序）。
- **禁止**：不动 MS18 零工作跳过与 MS26 水位判定；不引入 catalog 压缩/VACUUM；不改 `is_plain_column_expr` 对裸 NULL 字面量的排除；不为 D10 新增 env/config 通道或等待可调 API；不为验证新增判定层或身份型证据工程。

## 实现顺序与依赖

Iteration 000（存储层自包含）→ 001（planner 层，与 000 无代码耦合，顺序体现正确性优先；replan 扩入 T12 锁重试，与 T5-T8 无代码耦合、同属「修复假 `DatabaseLocked`/限定名解析两个静默错误面」的 Iteration 验收边界）→ 002（CLI/文档，消费前两者定稿语义）。000 内 T1（原语）→ T2（重构，依赖 T1）→ T3/T4（e2e 见证，依赖 T2）；001 内 T5（注册）→ T6（解析，依赖 T5）→ T7（ORDER BY，依赖 T5）→ T8（规格与校准收口）；T12 独立（存储锁分支，与 000 同文件面但不同符号，可与 T6-T8 并行或收尾执行）。
