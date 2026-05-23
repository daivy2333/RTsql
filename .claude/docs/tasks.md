# 任务追踪

> 最后更新：2026-05-23（M18 Phase2 Executor层非唯一索引测试覆盖 完成）

## 当前阶段：M17.5 代码清理 + 全面对比 ✅

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

---

## 当前阶段：M18 优化项目与技术债清理 ⏳

**设计文档**：`.claude/docs/superpowers/specs/2026-05-23-optimization-tech-debts-design.md`

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

**预估工期**：1天（实际完成）

---

### Phase3: WAL集成 + Group Commit（待开始）

- [ ] T1: 设计 WALRecord 结构
- [ ] T2: 实现 WALWriter + buffer管理
- [ ] T3: Group Commit策略实现
- [ ] T4: INSERT Executor 集成 WAL
- [ ] T5: 性能基准测试（验证 5-10x faster）
- [ ] T6: 崩溃恢复测试

**预估工期**：3-5天

---

### Phase4: B-Tree Merge（待开始）

- [ ] T1: LeafNode::merge 实现
- [ ] T2: BTree::delete 递归 merge回传
- [ ] T3: InternalNode merge处理
- [ ] T4: BufferPool.free_page 集成
- [ ] T5: btree_merge_test.rs 新增测试

**预估工期**：2-3天

---

## 下一步：Phase1 架构Warnings清理

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

### M15: SQLite 基础性能对比 ✅ (速度对比完成，全面对比未执行)

---

## 里程碑路线图

- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- **M17.5**: ⏳ 代码清理 + 全面对比
- M18: WAL 集成 + 写入优化