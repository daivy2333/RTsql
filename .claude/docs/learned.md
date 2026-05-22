# 学习记忆

> 最后更新：2026-05-22（M15 聚合函数与 GROUP BY 完成）

## API 路径速查

| API | 用途 | 位置 |
|-----|------|------|
| Database::open(path) | 打开/创建数据库 | database.rs |
| Database::execute_sql(sql) | 执行 SQL 语句 | database.rs |
| Database::close() | 关闭数据库 | database.rs |
| BufferPool::get_page(page_id) | 获取页（两阶段锁） | storage/buffer_pool.rs |
| PageGuard::page_data() | 零拷贝读取页数据 | storage/page_frame.rs |
| PageGuard::modify_page(f) | 修改页数据（自动 dirty） | storage/page_frame.rs |
| SlottedPageRef::new(&[u8]) | 只读零拷贝 slot 访问 | storage/page_format/slotted_page.rs |
| SlottedPage::delete_slot(i) | 删除 slot（含 compacting） | storage/page_format/slotted_page.rs |
| IndexManager::search(key) | Async search（无 spawn_blocking） | storage/btree/index_manager.rs |
| IndexManager::scan_all() | Async scan all | storage/btree/index_manager.rs |
| BTree::from_root(page_id, loader) | 临时实例（写操作） | storage/btree/btree.rs |
| PlanBuilder::build(stmt) | SQL → PhysicalPlan | parser/planner.rs |
| Pipeline::execute(database, sql) | 执行管道入口 | pipeline.rs |

## 文件速查

| 文件 | 作用 |
|------|------|
| src/database.rs | Database 协调器 |
| src/pipeline.rs | SQL 执行管道 |
| src/storage/buffer_pool.rs | BufferPool（两阶段锁） |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef + compacting |
| src/storage/btree/index_manager.rs | IndexManager（AtomicPageId + async） |
| src/executor/aggregate.rs | AggregateFunc + AggregateState + AggregateExecutor |
| src/executor/having.rs | HavingExecutor |
| src/executor/join.rs | JoinExecutor（哈希连接） |
| src/parser/planner.rs | PlanBuilder（含聚合解析） |

## 踩坑记录

| 问题 | 原因 | 解决 | 预防 |
|------|------|------|------|
| SlottedPage delete 不减少 slot_count | 只标记删除留空洞 | slot compacting | 删除操作必须更新 header |
| RwLock<BTree> 跨 .await | 死锁风险 | AtomicPageId | async 避免 std::sync::RwLock |
| search_from_page_async lifetime | Future 捕获 &self lifetime | Pin<Box<dyn Future + Send + 'a>> | async 递归显式标注 lifetime |
| criterion iterations 过多 | 100 个独立 case | 减至 50 个代表性 case | benchmark 避免大量独立 case |
| **extract_columns 遇到 Expr::Function** | 只处理 Identifier | 添加 Expr::Function 处理 | AST 辅助需覆盖所有 Expr 类型 |
| **HAVING 无法解析聚合列** | build_where 针对原始列 | build_having 针对聚合输出列 | HAVING 谓词必须用聚合输出列索引 |
| **AVG 整数除法** | Int/Int div 返回 Int | AVG 先转 f64 再除 | 聚合 AVG 必须返回 Float |
| **AggregateExecutor::new partial move** | Pipeline 解构 AggregateNode | 改为传单个字段 | 避免结构体解构导致 partial move |

## 技巧模式

| 模式 | 描述 | 适用场景 |
|------|------|----------|
| 零拷贝页读取 | page_data() + SlottedPageRef | 只读场景 |
| 零拷贝 BTree | page_data() + LeafNodeRef | BTree 读路径 |
| 两阶段锁 | 读锁→释放→I/O→写锁(double-check) | 缓存加载 |
| AtomicPageId | AtomicU64::load(Acquire) | async 无锁访问 |
| Hash Aggregation | HashMap<Vec<Value>, Vec<AggregateState>> | GROUP BY |
| HAVING 复用 | HavingExecutor 结构同 FilterExecutor | 聚合结果过滤 |
| 临时 BTree 实例 | BTree::from_root() + spawn_blocking | 写操作保持 sync |
| 哈希连接 | 构建侧哈希表 + 探测侧匹配 | INNER JOIN |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| WAL Group Commit | 中 | M18，INSERT 5-10x 提速 |
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
| B-Tree split/merge | 中 | M17 索引优化 |
| 子查询 | 中 | M16 |

## 详细踩坑档案

### extract_columns 遇到 Expr::Function

**症状**: 聚合函数出现在 SELECT 投影中时，extract_columns 只处理 Identifier，导致 parse 错误
**根因**: AST 辅助函数缺少 Expr::Function 分支
**解决**: extract_columns/extract_qualified_columns 添加 Expr::Function 处理，返回聚合结果列名
**预防**: AST 辅助需覆盖所有 sqlparser Expr 类型

### HAVING 无法解析聚合列

**症状**: `HAVING COUNT(*) > 5` 报错列找不到
**根因**: build_where 将列引用解析为原始表列（如 `count_star` 不在原始表中）
**解决**: 新增 build_having + build_having_expression，将列引用解析为聚合输出列索引
**预防**: HAVING 谓词中的列引用需映射到 AggregateNode 的 output_columns

### AVG 整数除法问题

**症状**: `AVG(90,80,70)` 返回 80（Int），而非 80.0（Float）
**根因**: Value::div 对 Int/Int 做整数除法
**解决**: AVG finalize 时先将 sum 转为 f64，再做浮点除法
**预防**: 聚合 AVG 结果类型必须是 Value::Float