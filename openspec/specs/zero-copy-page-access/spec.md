# zero-copy-page-access Specification

## Purpose
TBD - created by archiving change m20-zero-copy-slotted-page-ref. Update Purpose after archive.
## Requirements
### Requirement: BufferPool MUST 提供零拷贝只读页访问 API

`BufferPool` MUST 提供 `get_page_ref(page_id: PageId) -> Result<PageDataGuard<'_>>` 异步方法，调用方获得只读 guard 后可直接 `&guard[..]` 取得页数据 `&[u8]`，生命周期由 guard 持有，guard drop 时自动释放页锁。

#### Scenario: 成功获取零拷贝页引用
- **WHEN** 调用方传入有效 `PageId` 调用 `get_page_ref`
- **THEN** 返回的 `PageDataGuard` 可被 `Deref` 为 `&[u8]`，与 B-Tree 内部 `page_data()` 行为一致
- **AND** guard drop 后后续再次调用 `get_page_ref(same_id)` 可立即成功（不阻塞）

#### Scenario: 无效 PageId 返回错误
- **WHEN** 调用方传入未被缓冲池管理的 `PageId`
- **THEN** 返回 `StorageError::PageNotFound(page_id)`，不分配任何 guard

### Requirement: 数据页 tuple 读取 MUST 返回借用而非拷贝

`read_tuple_from_data_page` MUST 从 `(VersionHeader, Vec<u8>)` 改为借用形式，调用方在 guard 存活期间可对 `&[u8]` 执行 `deserialize_tuple` 等操作，零堆分配。

#### Scenario: 读取存在 tuple
- **WHEN** 给定有效 `RowId`（含 page_id + slot_id）
- **THEN** 返回 `(VersionHeader, &[u8])` 借用借用引用指向页内 slot 数据
- **AND** 调用方在 guard 仍存活时可对 `&[u8]` 调 `deserialize_tuple` 反序列化

#### Scenario: Slot 不存在返回错误
- **WHEN** 给定不存在的 `slot_id`
- **THEN** 返回 `StorageError::SlotNotFound(row_id)`，guard 在错误前已 drop

### Requirement: MVCC 版本链遍历 MUST 消除中间 Vec 分配

`BufferPool::find_visible_version` MUST 改为闭包形式，调用方在闭包作用域内消费 `&[u8]` 而非接收 `Vec<u8>` 返回值。

#### Scenario: 找到第一个可见版本
- **WHEN** 闭包内调用方对 `&[u8]` 反序列化
- **THEN** 返回 `Some(R)`，其中 `R` 由闭包返回（典型为 `Result<Vec<Value>, _>`）
- **AND** 函数本身不分配任何 `Vec<u8>` 持有元组数据

#### Scenario: 所有版本不可见
- **WHEN** 版本链遍历到 `next_version = None` 且均不可见
- **THEN** 返回 `Ok(None)`，闭包未被调用，guard 全部 drop

#### Scenario: 跨页遍历正确释放前页 guard
- **WHEN** 版本链跨多页（`next_version` 指向不同 page_id）
- **THEN** 每次迭代开始前前一页 guard 已 drop，新页 guard 在新一次 `.await` 后获取
- **AND** 不存在同时持有两个 page guard 的时刻

### Requirement: Scan 执行器 MUST 使用零拷贝 API

`ScanExecutor` / `IndexScanExecutor` / `IndexScanAllExecutor` 三个执行器的 tuple 读取路径 MUST 全部切换到 `get_page_ref` + 零拷贝 `read_tuple_from_data_page` 链路，消除 `Vec<u8>` 中间分配。

#### Scenario: 全表扫描零拷贝
- **WHEN** `ScanExecutor::next()` 处理 1K 行
- **THEN** 全程无 `Vec<u8>` 堆分配持有 tuple 字节（除 `deserialize_tuple` 内部 `Value` 分配）
- **AND** 编译后二进制调用栈中不再出现 `read_tuple_from_data_page` 返回 `Vec<u8>` 后的 `.clone()`

#### Scenario: 索引扫描零拷贝
- **WHEN** `IndexScanExecutor` / `IndexScanAllExecutor` 通过 `find_visible_version` 遍历
- **THEN** 闭包内 `&[u8]` 直接传递给 `deserialize_tuple`，无中间 `Vec<u8>` 拷贝

#### Scenario: 跨 await 持有借用安全
- **WHEN** 执行器在 `.await` 边界间持有 page data 借用
- **THEN** 通过 `Box::pin` 闭包模式围出 guard 作用域，编译通过无 borrow checker / Send 错误

### Requirement: 写路径 MUST 保持原状

`write_tuple_to_data_page` / `update_version_header_in_data_page` / `delete_tuple_from_data_page` 三个写路径函数 MUST NOT 被修改，保持原有 `Vec<u8>` 接口（如有），不影响 M10 既有行为。

#### Scenario: 写路径回归测试
- **WHEN** 跑 `tests/storage_test.rs` 全部测试
- **THEN** 所有写路径测试通过，行为与 M20 改前完全一致

#### Scenario: 写路径无零拷贝改动
- **WHEN** `git diff` 检查 src/storage/data_page.rs
- **THEN** 三个写函数（write/update/delete）函数体无变更
- **AND** 仅 `read_tuple_from_data_page` 函数体有变更（改返回借用）

### Requirement: 性能验收门槛

M20 完成后 MUST 达成 Full Scan 1K rows 提速 ≥ 15%（对比改前 baseline），其他基准回归 < 5%。

#### Scenario: Full Scan 提速达标
- **WHEN** 跑 `cargo bench --bench single -- --baseline before-m20` 对比
- **THEN** `full_scan_1k_rows` 基准中位数耗时下降 ≥ 15%
- **AND** 该套件内其他基准（INSERT / PK Lookup / DELETE 等）耗时变化 < 5%

#### Scenario: Micro 套件无回归
- **WHEN** 跑 `cargo bench --bench micro -- --baseline before-m20`
- **THEN** micro 套件内所有基准耗时变化 < 5%（无回归）

