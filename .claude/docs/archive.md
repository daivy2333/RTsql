# archive.md — 文档归档

> 自动归档记录，由 project-archivist 维护。
> 按源文档分节，每条含日期、编号、置信度、理由、恢复条件。
> 恢复方式: 用户说"恢复 §{文档名} #{编号}"。
> 搜索方式: grep "关键词" archive.md 或 grep "#编号" archive.md

---

## learned.md 归档

<!-- archive: learned #01 -->
**日期**: 2026-05-24
**条目**: SlottedPage delete 不减少 slot_count
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已修复且被 logical Row ID 根本性重构，旧描述不再准确
**恢复条件**: 需要回顾 SlottedPage 早期 delete 实现时

原始内容:
| SlottedPage delete 不减少 slot_count | 只标记删除留空洞 | slot compacting | 删除操作必须更新 header |

---

<!-- archive: learned #02 -->
**日期**: 2026-05-24
**条目**: RwLock<BTree> 跨 .await
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: AtomicPageId 方案已稳定运行 > 30d
**恢复条件**: 需要回顾 async + RwLock 死锁问题时

原始内容:
| RwLock<BTree> 跨 .await | 死锁风险 | AtomicPageId | async 避免 std::sync::RwLock |

---

<!-- archive: learned #03 -->
**日期**: 2026-05-24
**条目**: search_from_page_async lifetime
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已稳定方案，属于历史问题
**恢复条件**: 需要回顾 async 递归 lifetime 标注时

原始内容:
| search_from_page_async lifetime | Future 捕获 &self lifetime | Pin<Box<dyn Future + Send + 'a>> | async 递归显式标注 lifetime |

---

<!-- archive: learned #04 -->
**日期**: 2026-05-24
**条目**: criterion iterations 过多
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已在 M17.5 修复 > 30d
**恢复条件**: 需要回顾 criterion 基准测试优化时

原始内容:
| criterion iterations 过多 | 100 个独立 case | 减至 50 个代表性 case | benchmark 避免大量独立 case |

---

<!-- archive: learned #05 -->
**日期**: 2026-05-24
**条目**: extract_columns 遇到 Expr::Function
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已修复 > 30d
**恢复条件**: 需要回顾 AST 辅助函数 Expr 类型覆盖时

原始内容:
| extract_columns 遇到 Expr::Function | 只处理 Identifier | 添加 Expr::Function 处理 | AST 辅助需覆盖所有 Expr 类型 |

---

<!-- archive: learned #06 -->
**日期**: 2026-05-24
**条目**: HAVING 无法解析聚合列
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已修复 > 30d
**恢复条件**: 需要回顾 HAVING 谓词构建时

原始内容:
| HAVING 无法解析聚合列 | build_where 针对原始列 | build_having 针对聚合输出列 | HAVING 谓词必须用聚合输出列索引 |

---

<!-- archive: learned #07 -->
**日期**: 2026-05-24
**条目**: AVG 整数除法
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已修复 > 30d
**恢复条件**: 需要回顾聚合 AVG 实现时

原始内容:
| AVG 整数除法 | Int/Int div 返回 Int | AVG 先转 f64 再除 | 聚合 AVG 必须返回 Float |

---

<!-- archive: learned #08 -->
**日期**: 2026-05-24
**条目**: extract_columns/expr_to_column_name 遇到 Expr::Value
**原分类**: 踩坑记录
**置信度**: HIGH
**理由**: 已修复 > 30d
**恢复条件**: 需要回顾 Expr::Value 处理时

原始内容:
| **extract_columns 遇到 Expr::Value** | EXISTS 惯用 SELECT 1，Value 未被处理 | 添加 Expr::Value 分支返回字符串表示 | AST 辅助需覆盖所有 sqlparser Expr 类型 |
| **expr_to_column_name 不处理 Value** | 同上，SELECT 1 作为非聚合列处理失败 | 添加 Expr::Value 分支 | 确保列名提取与 extract_columns 覆盖一致 |

---

<!-- archive: learned #09-13 -->
**日期**: 2026-05-24
**条目**: 详细踩坑档案（extract_columns Expr::Function, HAVING, AVG, Expr::Value, get_subquery_first_column 旧版）
**原分类**: 详细踩坑档案
**置信度**: HIGH
**理由**: 已精简为表格行 + Simplified 档案，旧详细版归档
**恢复条件**: 需要完整踩坑细节时

---

<!-- archive: learned #14 -->
**日期**: 2026-05-24
**条目**: 待探索 — WAL Group Commit
**原分类**: 待探索
**置信度**: HIGH
**理由**: M18-Phase3 已完成，不再需要探索
**恢复条件**: 需要回顾 WAL Group Commit 原始规划时

原始内容:

| WAL Group Commit | 中 | M18，INSERT 5-10x 提速 |

---

<!-- archive: learned #15 -->
**日期**: 2026-05-24
**条目**: 待探索 — B-Tree split/merge
**原分类**: 待探索
**置信度**: HIGH
**理由**: M17-Phase2 + M18-Phase4 已完成，不再需要探索
**恢复条件**: 需要回顾 B-Tree 优化原始规划时

原始内容:

| B-Tree split/merge | 中 | M17 索引优化 |

---

## optimization.md 归档

<!-- archive: optimization #01 -->
**日期**: 2026-05-24
**条目**: PageGuard 零拷贝
**状态**: 已完成 (M13)
**置信度**: HIGH
**理由**: M13 已完成 > 90d
**恢复条件**: 需要回顾零拷贝实现细节时

原始内容:
| 1 | PageGuard 零拷贝 | M13 | scan/filter/sort 5-15% |

---

<!-- archive: optimization #02 -->
**日期**: 2026-05-24
**条目**: BufferPool 两阶段锁
**状态**: 已完成 (M13)
**置信度**: HIGH
**理由**: M13 已完成 > 90d
**恢复条件**: 需要回顾两阶段锁实现时

原始内容:
| 2 | BufferPool 两阶段锁 | M13 | 并发读 ~5% |

---

<!-- archive: optimization #03 -->
**日期**: 2026-05-24
**条目**: Plan Cache (LRU)
**状态**: 已完成 (M14)
**置信度**: HIGH
**理由**: M14 已完成 > 90d
**恢复条件**: 需要回顾 Plan Cache 实现时

原始内容:
| 3 | Plan Cache (LRU) | M14 | 相同 SQL 1.1x |

---

<!-- archive: optimization #04 -->
**日期**: 2026-05-24
**条目**: BTree 零拷贝读
**状态**: 已完成 (M14)
**置信度**: HIGH
**理由**: M14 已完成 > 90d
**恢复条件**: 需要回顾 BTree 零拷贝实现时

原始内容:
| 4 | BTree 零拷贝读 | M14 | PK 查询 1.2x |

---

<!-- archive: optimization #08 -->
**日期**: 2026-05-24
**条目**: INSERT 慢 — WAL Group Commit
**状态**: 已完成 (M18-Phase3)
**置信度**: HIGH
**理由**: WAL + Group Commit 已于 M18-Phase3 实现
**恢复条件**: 需要回顾 INSERT 优化原始规划时

原始内容:

| INSERT 慢 | ~440µs/行 | 5-10x 提速 | WAL Group Commit | M18 |

---

<!-- archive: optimization #09 -->
**日期**: 2026-05-24
**条目**: B-Tree Merge 未实现
**状态**: 已完成 (M18-Phase4)
**置信度**: HIGH
**理由**: B-Tree Merge 已于 M18-Phase4 实现
**恢复条件**: 需要回顾 Merge 原始规划时

原始内容:

| B-Tree Merge | 未实现 | 删除后 underflow | 页合并 + 页释放 | M18+ |

---

## tasks.md 归档

<!-- archive: tasks #01 -->
**日期**: 2026-05-24
**条目**: M17-Phase2: B-Tree Split 机制
**完成时间**: 2026-05-23
**置信度**: HIGH
**恢复条件**: 需要参考 Split 实现细节时

原始内容:
### M17-Phase2: B-Tree Split 机制 ✅ (2026-05-23)
- [x] T6: LeafNode::split 实现
- [x] T6: InternalNode::split 实现
- [x] T7: BTree::insert 递归 + split 回传
- [x] T8: 根分裂处理 + IndexManager root_page_id 更新
- [x] T8: InternalNodeRef find_child_page_id 路由修复
- [x] T9: 测试套件（7 个场景覆盖）

---

<!-- archive: tasks #02 -->
**日期**: 2026-05-24
**条目**: M17-Phase1: 非唯一索引
**完成时间**: 2026-05-23
**置信度**: HIGH
**恢复条件**: 需要参考非唯一索引实现时

原始内容:
### M17-Phase1: 非唯一索引 ✅ (2026-05-23)
- [x] T1: NonUniqueIndex 模式 + DuplicateKey 处理
- [x] T2: search_all / scan_all 支持
- [x] T3: delete_by_key / delete_exact
- [x] T4: SplitResult 结构体
- [x] T5: InternalNode::insert_separator

---

<!-- archive: tasks #03 -->
**日期**: 2026-05-24
**条目**: M16: 子查询支持
**完成时间**: 2026-05-22
**置信度**: HIGH
**恢复条件**: 需要参考子查询实现时

原始内容:
### M16: 子查询支持 ✅

---

## architecture.md 归档

<!-- architecture.md entries below -->

---

## SNAPSHOT.md 归档

<!-- archive: snapshot #01 -->
**日期**: 2026-05-24
**条目**: M17-Phase2 历史快照 + 旧提交列表
**置信度**: HIGH
**理由**: 历史快照，当前 Phase3 已覆盖
**恢复条件**: 需要查看 M17 提交历史时

原始内容:
M17-Phase2 新增功能表 + M17 提交列表 (72c69dc, f54a6c7, 95b60b2, d3a7c0c, 238d9a7)

---

## references.md 归档

<!-- references.md entries below -->
