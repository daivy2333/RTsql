# 学习记忆

> 最后更新：2026-05-23（M17-Phase1 非唯一索引 完成）

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
| inject_correlated_values(plan, values) | 向谓词树注入外层列值 | executor/correlated.rs |
| ParameterExpression::new(name) | 创建可注入参数占位符 | executor/predicate.rs |
| predicate.inject_parameters(values) | 递归注入参数值到谓词树 | executor/predicate.rs |
| **LeafNodeRef::find_all_matches(key)** | 查找所有匹配 key 的 slot 索引 | storage/btree/node.rs |
| **BTree::search_all(key)** | 返回所有匹配 RowId（非唯一索引） | storage/btree/btree.rs |
| **BTree::delete_by_key(key)** | 删除所有匹配 entries（返回数量） | storage/btree/btree.rs |
| **BTree::delete_exact(key, row_id)** | 精确删除（key + RowId 匹配） | storage/btree/btree.rs |
| **InternalNode::insert_separator(key, right_child)** | 插入分隔符（用于 split） | storage/btree/node.rs |
| **LeafNode::delete_slot(index)** | 按索引删除 slot（公开方法） | storage/btree/node.rs |

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
| src/executor/semi_join.rs | SemiJoinExecutorV2（独立+关联双路径） |
| src/executor/anti_join.rs | AntiJoinExecutor（独立+关联双路径） |
| src/executor/subquery_eval.rs | SubqueryEvalExecutor（独立+关联双路径） |
| src/executor/correlated.rs | inject_correlated_values 注入函数 |
| src/executor/predicate.rs | Predicate/Expression trait + ParameterExpression |
| src/parser/planner.rs | PlanBuilder（含子查询/关联检测） |
| src/parser/ast.rs | extract_columns/extract_qualified_columns |

## 踩坑记录

| 问题 | 原因 | 解决 | 预防 |
|------|------|------|------|
| SlottedPage delete 不减少 slot_count | 只标记删除留空洞 | slot compacting | 删除操作必须更新 header |
| RwLock<BTree> 跨 .await | 死锁风险 | AtomicPageId | async 避免 std::sync::RwLock |
| search_from_page_async lifetime | Future 捕获 &self lifetime | Pin<Box<dyn Future + Send + 'a>> | async 递归显式标注 lifetime |
| criterion iterations 过多 | 100 个独立 case | 减至 50 个代表性 case | benchmark 避免大量独立 case |
| extract_columns 遇到 Expr::Function | 只处理 Identifier | 添加 Expr::Function 处理 | AST 辅助需覆盖所有 Expr 类型 |
| HAVING 无法解析聚合列 | build_where 针对原始列 | build_having 针对聚合输出列 | HAVING 谓词必须用聚合输出列索引 |
| AVG 整数除法 | Int/Int div 返回 Int | AVG 先转 f64 再除 | 聚合 AVG 必须返回 Float |
| AggregateExecutor::new partial move | Pipeline 解构 AggregateNode | 改为传单个字段 | 避免结构体解构导致 partial move |
| **extract_columns 遇到 Expr::Value** | EXISTS 惯用 SELECT 1，Value 未被处理 | 添加 Expr::Value 分支返回字符串表示 | AST 辅助需覆盖所有 sqlparser Expr 类型 |
| **expr_to_column_name 不处理 Value** | 同上，SELECT 1 作为非聚合列处理失败 | 添加 Expr::Value 分支 | 确保列名提取与 extract_columns 覆盖一致 |
| **get_subquery_first_column 不支持 SemiJoin** | 嵌套 IN 子查询 Plan 是 SemiJoin 节点 | 添加 SemiJoin/AntiJoin 分支，使用 output_columns | Plan 递归提取函数需覆盖所有带 output_columns 的节点 |
| **多层检测永远不触发** | 1089 行 get_subquery_first_column 在 1100 行 extract_correlated_params 之前调用，且未处理 SemiJoin | 修复 get_subquery_first_column 让流程走到 extract_correlated_params | 检查顺序敏感的函数需确保上游验证不影响下游逻辑 |
| **inner_column_index 设计错误** | 设为 usize 表示"替换位置索引"，但 ColumnExpression 在谓词树非输出列 | 改为 param_name: String，按列名匹配 ParameterExpression | 设计相关子查询注入时用名称匹配而非索引匹配 |

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
| **Mutex 参数注入** | ParameterExpression 携带 Mutex<Value>，clone plan → inject → rebuild executor | 相关子查询外层值注入 |
| **双路径执行器** | correlated_params 非空走按行重建路径，空走预构建快速路径 | SemiJoin/AntiJoin/SubqueryEval |
| **inner_table_names 上下文** | PlanBuilder 字段，build_query 前设置子查询表列表，build_expression 据此创建 ParameterExpression | 外部引用检测 |
| **非唯一索引同页多条目** | LeafNode 去掉 DuplicateKey 检查，find_all_matches 遍历所有匹配 slot | 索引允许重复 key |
| **批量删除从后向前** | delete_by_key matches 从后向前删除 slot，避免索引错位 | 批量删除同页多个 slot |
| **两次加载页模式** | 先 page_data() 读取找匹配，再 modify_page() 删除，避免闭包借用冲突 | 页面读写分离操作 |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| WAL Group Commit | 中 | M18，INSERT 5-10x 提速 |
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
| B-Tree split/merge | 中 | M17 索引优化 |

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

### extract_columns / expr_to_column_name 遇到 Expr::Value

**症状**: `EXISTS (SELECT 1 FROM ...)` 返回 "Unsupported statement type"
**根因**: extract_columns、extract_qualified_columns、expr_to_column_name 三处均未处理 Expr::Value（SQL 惯用 `SELECT 1`）
**解决**: 三处均添加 Expr::Value 分支，返回 value 字符串表示作为列名
**预防**: 每新增一种 Expr 类型到系统时，需在所有 AST 辅助函数中同步更新

### get_subquery_first_column 不支持 SemiJoin/AntiJoin

**症状**: 嵌套 IN 子查询（中间层含 SemiJoin）返回 "Subquery returns multiple columns"
**根因**: get_subquery_first_column 在 extract_correlated_params（多层检测）之前调用，且仅处理 Scan/Filter/Aggregate
**解决**: 添加 SemiJoin/AntiJoin 分支，使用 output_columns 字段提取首列
**预防**: Plan 递归提取函数需覆盖所有带 output_columns 的节点类型

### inner_column_index 设计失误

**症状**: CorrelatedParam 中用 usize 表示"替换位置"，但 ColumnExpression 在谓词树内不在输出列中
**根因**: 设计时未区分"内层输出列索引"和"谓词树内匹配"两个概念
**解决**: 重构为 param_name: String，注入时按列名遍历谓词树匹配 ParameterExpression
**预防**: 设计相关子查询注入机制时首选名称匹配而非索引匹配，避免位置语义歧义
