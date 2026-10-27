# 语料交接

收尾合并时写入主规格。本 Cycle 不改 `openspec/specs/`。

## 索引读路径的五个标题

域目录：`index-read-bounded-read`。

落位顺序：合并时先按本变更的增量 `specs/index-read-bounded-read/spec.md` 套用。增量里仍是下面的旧标题，当前主规格里的每个 Scenario 标题才能在 `MODIFIED` 块里逐字出现，归档校验才会通过。套用之后，再在合并后的主规格上做下面五处替换。先改主规格标题会使归档拒绝该增量。

| 种类 | 当前标题 | 替换为 |
|---|---|---|
| Requirement | 损坏的索引页在读 / 写路径上给出明确的错误 | 未通过页级判据的索引树页在读 / 写路径上给出明确的错误 |
| Scenario | 损坏的索引页上的查询不 panic、不静默错读 | 未通过页级判据的索引树页上的查询不 panic、不静默错读 |
| Scenario | 损坏的索引页上的插入不 panic | 未通过页级判据的索引树页上的插入不 panic |
| Scenario | 损坏的索引页上的删除不 panic | 未通过页级判据的索引树页上的删除不 panic |
| Scenario | 损坏的索引页上的更新不 panic | 未通过页级判据的索引树页上的更新不 panic |

不改 SHALL 正文，不改 GIVEN / WHEN / THEN，不改 Scenario 数量，不改 Requirement 数量。下面两个标题不动：

- 未损坏路径的读 / 写行为零变化
- 叶页条目长度过短的索引树页给出明确的错误

GIVEN 里的「页内容已损坏」是注入页的描述，不在这五处替换里。

## 13 份占位 Purpose

落位时机与上一节相同：收尾合并时把下面的新 Purpose 整段替换进对应主规格的当前占位句。本 Cycle 不写 `openspec/specs/`。每条新 Purpose 只复述该域 Requirements 里已有的能力，并带上占位句中的 change id。

### cli-noninteractive-shell

当前占位：

```text
TBD - created by archiving change 2026-09-06-ms10-t01-cli-shell. Update Purpose after archive.
```

新 Purpose：

```text
`rtsql` 提供参数化 one-shot 主命令以及生命周期和分析子命令：打开数据库、执行 SQL、把结果写到标准输出，并以分类退出码退出。来源：`2026-09-06-ms10-t01-cli-shell`
```

### database-file-format-header

当前占位：

```text
TBD - created by archiving change 2026-09-08-ms10-t03-file-format-header. Update Purpose after archive.
```

新 Purpose：

```text
主数据库文件以固定 64 字节头开始；打开空文件时写入该头，打开非空文件时校验该头，格式不符则在页解析之前按原因拒绝。来源：`2026-09-08-ms10-t03-file-format-header`
```

### database-file-lock

当前占位：

```text
TBD - created by archiving change 2026-09-06-ms10-t02-file-lock-graceful-shutdown. Update Purpose after archive.
```

新 Purpose：

```text
读写打开对主数据库文件取得非阻塞 advisory 独占锁，锁被占用时按等待预算重试，预算耗尽后返回 `DatabaseLocked`；只读打开对共享锁使用同一等待预算。来源：`2026-09-06-ms10-t02-file-lock-graceful-shutdown`
```

### dml-transaction-lifecycle

当前占位：

```text
TBD - created by archiving change 2026-08-25-fix-dml-tx-id-injection. Update Purpose after archive.
```

新 Purpose：

```text
经 `Database::execute_sql` 发出的 INSERT、UPDATE 和 DELETE 都运行在 `TransactionManager` 的真实事务中，成功则提交，失败则中止，提交时保留删除墓碑。来源：`2026-08-25-fix-dml-tx-id-injection`
```

### drop-table-physical-free

当前占位：

```text
TBD - created by archiving change 2026-08-26-ms07-t02-drop-table-physical-free. Update Purpose after archive.
```

新 Purpose：

```text
`TableManager::drop_table` 在抹除目录行并移出内存表项之后，把该表的数据页和 BTree 页归还到 `FileStorage::free_pages`。来源：`2026-08-26-ms07-t02-drop-table-physical-free`
```

### ms07-rest-tx-checkpoint-pushdown

当前占位：

```text
TBD - created by archiving change 2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown. Update Purpose after archive.
```

新 Purpose：

```text
`Database` 提供 begin、commit 和 rollback，使多条语句处于同一事务；恢复消费 checkpoint 位点并跳过已被覆盖的 WAL 前缀；扫描可在迭代内过滤谓词，无排序的 LIMIT 可下推进扫描并提前停止。来源：`2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown`
```

### pipeline-stage-decomposition

当前占位：

```text
TBD - created by archiving change 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages. Update Purpose after archive.
```

新 Purpose：

```text
SQL 执行拆成 parse、plan、execute 三个阶段函数，完整流程由这三段按顺序组成。来源：`2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`
```

### plancache-key-normalization

当前占位：

```text
TBD - created by archiving change 2026-08-25-ms06-t02-plancache-dashmap. Update Purpose after archive.
```

新 Purpose：

```text
PlanCache 存取时规范化 SQL 文本，只把语义相关的差异识别为不同 key，并发读取同一 SQL 时不阻塞运行时任务。来源：`2026-08-25-ms06-t02-plancache-dashmap`
```

### planner-module-decomposition

当前占位：

```text
TBD - created by archiving change 2026-08-30-ms07-t03-planner-decomposition. Update Purpose after archive.
```

新 Purpose：

```text
`src/parser/planner.rs` 拆成六个职责模块，`PlanBuilder` 的对外 API 和方法签名保持不变。来源：`2026-08-30-ms07-t03-planner-decomposition`
```

### schema-persistence

当前占位：

```text
TBD - created by archiving change 2026-08-26-ms07-t01-schema-persistence. Update Purpose after archive.
```

新 Purpose：

```text
表定义写入系统表 `__tables` 与 `__columns`，`Database::open` 读取这两张表，在进程重启后重建全部 `TableMeta`。来源：`2026-08-26-ms07-t01-schema-persistence`
```

### storage-io-optimization

当前占位：

```text
TBD - created by archiving change 2026-09-05-ms08-t01-t02-pread-prefetch. Update Purpose after archive.
```

新 Purpose：

```text
页读写使用 `read_exact_at` / `write_all_at`，每次页 I/O 不再 seek；数据页链预取默认关闭，显式启用时不改变行序和查询结果。来源：`2026-09-05-ms08-t01-t02-pread-prefetch`
```

### wal-recovery-frame-parsing

当前占位：

```text
TBD - created by archiving change 2026-09-06-ms10-t02-file-lock-graceful-shutdown. Update Purpose after archive.
```

新 Purpose：

```text
WAL 逐帧解析无歧义且自验证：新格式以 CRC 为接受判据，歧义偏移上新旧格式都会尝试并恰好一种按其自身规则成功，两条路径都失败则显式报错。来源：`2026-09-06-ms10-t02-file-lock-graceful-shutdown`
```

### wal-writer-handle-reuse

当前占位：

```text
TBD - created by archiving change 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages. Update Purpose after archive.
```

新 Purpose：

```text
`WalWriter` 持有持久文件句柄，写入、fsync、截断和读取当前 LSN 都通过该句柄完成，不随每次调用打开关闭文件。来源：`2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`
```

## 不替换

`wal-recovery-replay-integrity` 的 Purpose 已有正文，文中没有 TBD，也没有 TODO。本交接不提供替换句。收尾合并时保留现文，不覆盖。
