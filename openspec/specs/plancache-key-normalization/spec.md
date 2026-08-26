# plancache-key-normalization Specification

## Purpose
TBD - created by archiving change 2026-08-25-ms06-t02-plancache-dashmap. Update Purpose after archive.
## Requirements
### Requirement: PlanCache key 规范化

PlanCache SHALL 在 put/get 时对 SQL 文本做规范化，保证仅语义相关的差异被识别为不同 key。

#### Scenario: 大小写变体 hit

- **WHEN** 客户端以 `SELECT * FROM T` 写入 cache
- **AND** 第二次以 `select * from t` 查询
- **THEN** 第二次查询命中同一 entry

#### Scenario: 空白变体 hit

- **WHEN** 客户端以 `SELECT * FROM t` 写入 cache
- **AND** 第二次以 `SELECT\n*\nFROM t` 查询
- **THEN** 第二次查询命中同一 entry

#### Scenario: 字符串字面量大小写区分

- **WHEN** 客户端以 `WHERE name = 'alice'` 写入 cache
- **AND** 第二次以 `WHERE name = 'Alice'` 查询
- **THEN** 第二次查询**不命中**（字符串字面量大小写不同视为不同语义）

#### Scenario: 字符串字面量内容保留

- **WHEN** 规范化函数处理 `WHERE name = 'SELECT'`
- **THEN** 输出 key 仍含 `WHERE name = 'SELECT'`（字符串字面量内不 lowercase）

### Requirement: PlanCache 并发无锁访问

PlanCache SHALL 在 100 并发 SELECT 同一 SQL 的场景下不阻塞 tokio runtime 任务。

#### Scenario: 100 并发同 SELECT 全部 hit

- **WHEN** 100 个并发任务同时执行相同 SQL（cache 已预热）
- **THEN** 全部返回正确结果
- **AND** 总耗时 < 5s（在标准开发机上）
- **AND** runtime worker 不出现 lock contention 死锁

#### Scenario: 100 并发不同 SQL 写入

- **WHEN** 100 个并发任务同时 put 不同 SQL
- **THEN** cache 接受全部写入（受 max_size 限制则驱逐旧 entry）
- **AND** 全部 put 不发生 panic 或 race condition

### Requirement: DML 与 DDL 行为保持

PlanCache SHALL 维持现有对 DML 不缓存、对 DDL 清空的行为。

#### Scenario: DML 不进 cache

- **WHEN** 客户端执行 `INSERT/UPDATE/DELETE`
- **THEN** cache size 不增加

#### Scenario: DDL 清空 cache

- **WHEN** 客户端执行 `CREATE/DROP TABLE`
- **THEN** cache size 变为 0

### Requirement: Database 持有者类型

Database 字段 `plan_cache` 的类型 SHALL 为 `Arc<PlanCache>`，无外层 `std::sync::Mutex`。

#### Scenario: Database 字段类型

- **WHEN** 查看 `Database` 结构体
- **THEN** `plan_cache: Arc<PlanCache>`（不包含 `Mutex`）

#### Scenario: PlanCache API 形态

- **WHEN** 调用 `PlanCache::get` / `put` / `clear` / `len` / `is_empty`
- **THEN** 全部方法签名为 `&self`（无 `&mut self`）

