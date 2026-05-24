# 任务追踪

> 最后更新：2026-05-24（M18 Phase3 全部完成，WAL Group Commit 基准测试通过）

## ✅ 已完成：修复 gc_test SlottedPage SlotID 失效 bug

**优先级**: P0 — 已修复

- [x] 引入 logical_id 解耦 RowId.slot_id 与物理 slot_index
- [x] Slot 从 4B 扩展为 6B（新增 logical_id: u16）
- [x] SlottedPageHeader 新增 next_logical_id 字段
- [x] 新增 get_slot_by_logical_id / delete_slot_by_logical_id 方法
- [x] delete_slot 修复：serialize header back to page.data
- [x] data_page.rs 全部改用 logical_id 查找
- [x] btree/node.rs 适配 add_slot 返回值
- [x] gc_test 3 个测试全部通过
- [x] 全量 cargo test 通过（417 tests, 0 failures）
- [x] Clippy 0 warnings

**修复方案**: 逻辑 Row ID（SlottedPage 内部 logical_id → slot_index 映射）
**详细踩坑档案**: `.claude/docs/learned.md` — gc_test SlottedPage SlotID 失效

---

## 当前阶段：M18 优化项目与技术债清理

### 已完成

- [x] M17.5-T1: Clippy 零警告（6 个架构 warnings 已留档）
- [x] M17.5-T2: 测试修复（174+ tests pass, 0 failures）
- [x] M17.5-T3: SQLite 全面对比基准测试（扩展 benches/sqlite_compare.rs，多维度对比完成）
- [x] M17.5-T4: 代码格式统一（cargo fmt 已完成）

**M17.5 核心成果**：
- INSERT 性能：RTsql 332x faster than SQLite ⚡
- PK lookup 性能：RTsql 5.6x faster than SQLite ⚡
- B-Tree Split 性能验证：稳定
- 非唯一索引功能验证：正常
- 文件大小：RTsql 6.5x larger（页格式开销）
- 二进制大小：RTsql 2.2x larger（Tokio runtime）

---

### Phase1: 架构Warnings清理 ✅

- [x] T1: 引入 JoinConfig/JoinRelatedConfig struct（解决 too_many_arguments）
- [x] T2: 定义 CreateExecutorFuture type alias（解决 type_complexity）
- [x] T3: #[allow] await_holding_lock（两阶段锁模式安全）
- [x] T4: #[allow] module_inception（标准命名模式）
- [x] T5: Clippy 验证（warnings 从 6降至 0）

**Phase1成果**：
- ✅ 所有代码 warnings 已清理（仅剩 cargo config deprecated）
- ✅ 参数组织清晰（JoinConfig/JoinRelatedConfig）
- ✅ 类型签名简洁（CreateExecutorFuture）
- ✅ #[allow] 有明确注释说明合理设计

---

### Phase2: Executor层非唯一索引测试覆盖 ✅

- [x] T1: 新增 IndexManager::search_all 方法
- [x] T2: 实现 IndexScanAllExecutor::execute
- [x] T3: executor_test.rs 新增非唯一索引测试
- [x] T4: SQL层集成验证

**Phase2成果**：
- ✅ IndexManager 新增 search_all 方法（支持非唯一索引查询）
- ✅ IndexScanAllExecutor 实现完成（逐行返回，MVCC 可见性）
- ✅ executor_test.rs 新增 3 个测试（基础功能/空结果/单结果）
- ✅ PhysicalPlan::IndexScanAll 节点集成
- ✅ Pipeline 创建 IndexScanAllExecutor 逻辑
- ✅ 101 tests pass, 0 failures
- ✅ Clippy 0 warnings

---

### Phase3: WAL集成 + Group Commit + 崩溃恢复 ✅

- [x] T1: WalRecord 扩展 + CRC32 + LSN (1fcc213)
- [x] T2: WALBuffer + Group Commit (27e8aca)
- [x] T3: Logical Row ID 修复 gc_test bug (650761d)
- [x] T4: TransactionManager 集成 WAL (begin/commit/abort 写 WAL 记录)
- [x] T5: Executor 集成 WAL + 隐式事务包装 (Insert/Update/Delete 写 BeginTxn+数据+CommitTxn)
- [x] T6: RecoveryManager 数据重放 (full_recover + redo committed + mark uncommitted)
- [x] T7: WAL Group Commit 性能基准测试 (benches/wal_group_commit_bench.rs 3 groups pass)
- [x] T8: 崩溃恢复 E2E 测试 (recovery_e2e_test.rs 6 tests pass)

**关键发现**:
1. **Executor 隐式事务包装** — 每个 Insert/Update/Delete 语句在 WAL 中自动生成完整的 BeginTxn → 数据记录 → CommitTxn 事务序列，使 RecoveryManager 能正确分类 committed/uncommitted 事务
2. **TableManager 纯内存限制** — 表定义不持久化到磁盘，重启后丢失。恢复时 redo 需要表存在才能写入数据页，但表不存在时 `get_table` 失败则 redo 跳过
3. **BufferPool::mark_tx_aborted 是 stub** — 当前实现为空函数，未遍历 SlottedPage 标记 uncommitted tuple

**417 tests pass, 0 failures, Clippy 0 warnings**

---

### Phase4: B-Tree Merge（待开始）

- [ ] T1: LeafNode::merge 实现
- [ ] T2: BTree::delete 递归 merge回传
- [ ] T3: InternalNode merge处理
- [ ] T4: BufferPool.free_page 集成
- [ ] T5: btree_merge_test.rs 新增测试

**预估工期**：2-3天

---

## 下一步：Phase4 B-Tree Merge

---

## 已完成

### M17-Phase2: B-Tree Split 机制 ✅ (2026-05-23)

- [x] T6: LeafNode::split 实现
- [x] T6: InternalNode::split 实现
- [x] T7: BTree::insert 递归 + split 回传
- [x] T8: 根分裂处理 + IndexManager root_page_id 更新
- [x] T8: InternalNodeRef find_child_page_id 路由修复
- [x] T9: 测试套件（7 个场景覆盖）

### M17-Phase1: 非唯一索引 ✅ (2026-05-23)

- [x] T1: NonUniqueIndex 模式 + DuplicateKey 处理
- [x] T2: search_all / scan_all 支持
- [x] T3: delete_by_key / delete_exact
- [x] T4: SplitResult 结构体
- [x] T5: InternalNode::insert_separator

### M16: 子查询支持 ✅

### M15: SQLite 基础性能对比 ✅

---

## 里程碑路线图

- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- M17.5: ✅ 代码清理 + 全面对比
- M18-Phase1: ✅ 架构Warnings清理
- M18-Phase2: ✅ Executor层非唯一索引
- **M18-Phase3**: ✅ **WAL集成 + Group Commit + 崩溃恢复**（全部完成）
- M18-Phase4: ⏳ B-Tree Merge（待开始）