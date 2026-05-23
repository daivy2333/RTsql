# 任务追踪

> 最后更新：2026-05-23（M17.5 清理阶段 规划中）

## 当前阶段：M17.5 代码清理 + 全面对比

### M17.5-T1: Clippy 零警告

**目标**: `cargo clippy -- -D warnings` 通过，0 warnings

**子任务**:
- [ ] M17.5-T1a: 简单修复（io_other_error、clone_on_copy、redundant_closure、into_iter、to_string_in_format、explicit_auto_deref、byte_str、single_match）~30 处
- [ ] M17.5-T1b: 中等修复（too_many_arguments 参数重构、await_holding_lock buffer_pool 重构、only_used_in_recursion）~7 处
- [ ] M17.5-T1c: 评估修复（dead_code 是否删除、module_inception 是否重命名）~3 处

### M17.5-T2: 测试修复

**目标**: `cargo test` 0 failures，0 compilation errors

**子任务**:
- [ ] M17.5-T2a: 修复 test_btree_insert_duplicate_key_returns_error（更新为测试非唯一索引行为）
- [ ] M17.5-T2b: 修复 planner_test.rs 编译错误（19 个 builder mutability 问题）
- [ ] M17.5-T2c: 添加 M17 新功能的 SQL 层集成测试（非唯一索引 + split）

### M17.5-T3: SQLite 全面对比基准测试

**目标**: 编写全面的基准测试，对比 RTsql vs SQLite 在多维度上的表现

**子任务**:
- [ ] M17.5-T3a: 编写基准测试脚本（内存、启动时间、文件大小、编译产物大小、加载性能、并发资源消耗）
- [ ] M17.5-T3b: 运行基准测试并记录结果
- [ ] M17.5-T3c: 分析结果，更新 optimization.md

### M17.5-T4: 代码格式统一

**目标**: `cargo fmt --check` 通过

**子任务**:
- [ ] M17.5-T4a: 运行 `cargo fmt` 统一格式
- [ ] M17.5-T4b: 检查并确认无意外格式变更

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