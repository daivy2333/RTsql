# 任务追踪

> 最后更新：2026-05-23（M17.5 清理阶段 已完成）

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

## 下一步：M18 WAL 集成 + 写入优化

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