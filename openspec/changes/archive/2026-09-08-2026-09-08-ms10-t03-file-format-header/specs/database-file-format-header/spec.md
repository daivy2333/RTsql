# database-file-format-header Specification

## ADDED Requirements

### Requirement: 文件头布局与生命周期

主数据库文件 SHALL 以固定 64 字节头开始：magic `"RTSQLDB\0"`（8B）、format_version u32 LE、flags u32 LE、page_size u32 LE、32B 预留盐区、12B 保留区。`FileStorage::open` SHALL 在打开 0 字节文件（新库）时立即写入头；SHALL 在打开非空文件时校验头；页 N 的文件偏移 SHALL 为 `64 + N * page_size`。头位于页空间之外，`PageId::to_offset` 的纯数学语义不变。`AsyncStorage::page_count()` SHALL 继续只统计页数（header-only 文件返回 0），保持 `TableManager::new` 以 `page_count()==0` 触发 catalog bootstrap 的既有契约。

#### Scenario: 新库创建即带头

- **GIVEN** 目标路径不存在或为 0 字节文件
- **WHEN** `Database::open` 打开该路径
- **THEN** 文件以 64B 头开始（magic/version=1/flags=0/page_size=4096），随后 catalog bootstrap 照常分配页 0 和页 1，`page_count()` 在 bootstrap 前为 0

#### Scenario: 既有 v1 库重开校验通过

- **GIVEN** 由本格式创建的数据库文件（含头 + 页）
- **WHEN** 再次 `Database::open`
- **THEN** magic、version、flags、page_size 全部校验通过，页数据经 `64 + N*4096` 偏移访问，SQL 行为与加头前一致

#### Scenario: to_offset 纯数学语义不变

- **GIVEN** `PageId(5)` 与 page_size 4096
- **WHEN** 调用 `PageId::to_offset(4096)`
- **THEN** 返回 20480（不含头偏移；头偏移只由 FileStorage 的页 I/O 调用点合成）

### Requirement: 格式错误显式拒绝

`FileStorage::open` SHALL 在任何页解析、WAL 打开或恢复之前完成头校验，并按原因分类报错：头不完整或 magic 不符（含 version=0）→ `StorageError::NotADatabase`；version 大于当前支持版本 → `StorageError::NewerFileVersion`（消息表明"文件由新版创建"）；未知 flags 位（含加密位）或 page_size 与本构建不符 → `StorageError::IncompatibleHeader`；扣除头后文件长度非页整除 → 既有 `StorageError::PageSizeMismatch`。以上均为 additive 变体，CLI 侧经既有 General 分支映射退出码 1 并输出具体原因到 stderr。

#### Scenario: 大小合法的垃圾文件干净拒绝（panic 回归）

- **GIVEN** 一个 8192 字节的随机数据文件（当前实现：SlottedPage 解析 panic → abort，exit 134）
- **WHEN** `rtsql ./garbage.db "SELECT 1"` 打开该文件
- **THEN** 进程不 panic、不 abort，以退出码 1 结束，stderr 报"不是 RTsql 数据库"及路径，文件内容未被修改

#### Scenario: 文件由新版创建

- **GIVEN** 一个 magic 匹配但 format_version 高于当前支持版本的文件（测试构造）
- **WHEN** `Database::open` 打开
- **THEN** 返回 `NewerFileVersion`，CLI 以退出码 1 报"文件由新版 RTsql 创建"

#### Scenario: 未知特性 flag 拒绝（加密库前向防护）

- **GIVEN** 一个 magic 匹配、version=1 但 flags 含当前版本未知的位（如加密位）的文件
- **WHEN** `Database::open` 打开
- **THEN** 返回 `IncompatibleHeader`，不把密文页当明文解析

#### Scenario: 截断文件拒绝

- **GIVEN** 一个有效头后跟随非整页字节数的文件（如 64 + 100 字节）
- **WHEN** `Database::open` 打开
- **THEN** 返回 `PageSizeMismatch`，错误信息可判读

#### Scenario: 过小文件拒绝

- **GIVEN** 一个非空但小于 64 字节的文件（如 57B 文本）
- **WHEN** `Database::open` 打开
- **THEN** 返回 `NotADatabase`（替代此前的误导性 `Page size mismatch`）

#### Scenario: 旧无头文件统一拒绝

- **GIVEN** 一个 T03 之前创建的无头库文件（页 0/1 直接位于文件头）
- **WHEN** `Database::open` 打开
- **THEN** 返回 `NotADatabase`，不做旧格式嗅探与迁移（用户决策 2026-09-08）

### Requirement: 打开顺序守卫

`FileStorage::open` SHALL 按固定顺序执行：独占锁 → 头校验/初始化 → 页长度校验 → 返回。锁 SHALL 先于头校验——文件被其他持有者占用时，即使文件内容不是合法 RTsql 格式，第二打开者得到的也是 `DatabaseLocked`（退出码 4）。头校验拒绝时 SHALL NOT 触碰 WAL、checkpoint 位点或任何页数据。

#### Scenario: 锁优先于头校验

- **GIVEN** 进程 A 已持有某个非 RTsql 文件的 flock
- **WHEN** 进程 B 尝试打开同一路径
- **THEN** B 得到 `DatabaseLocked`（退出码 4），而非格式错误（退出码 1）

#### Scenario: 拒绝时不触碰伴生文件

- **GIVEN** 一个 magic 不符的文件
- **WHEN** `Database::open` 失败
- **THEN** 同目录未创建或未修改 `<db>.wal` / `<db>.checkpoint`

### Requirement: 既有语义零回归

除头引入的变化外，既有打开/创建语义 SHALL 保持：0 字节文件仍是"新库"的唯一入口；`drop_table` 后文件长度、页计数复用行为经 helper 扣头后语义不变；页 I/O 越界读仍报 `UnexpectedEof`；WAL 与 checkpoint 位点文件的创建、消费、截断行为零变化。

#### Scenario: 0 字节新库契约保持

- **GIVEN** 0 字节文件与既有存储层测试套件（约 30 处 `FileStorage::open` 空文件调用）
- **WHEN** 打开并执行 allocate/read/write
- **THEN** 首分配页仍为 PageId(0)，`page_count()` 递增语义不变，既有测试断言除显式扣头 helper 外零修改通过

#### Scenario: 伴生文件行为不变

- **GIVEN** 任意主库打开/关闭周期
- **WHEN** 观察同目录伴生文件
- **THEN** `.wal` 于 open 时创建、checkpoint 时重写截断，`.checkpoint` 于 checkpoint 时写入 16B 位点——与加头前一致（文件长度差仅来自主库文件的 64B 头）
