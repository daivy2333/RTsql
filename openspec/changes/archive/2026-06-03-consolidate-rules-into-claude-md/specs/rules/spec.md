## REMOVED Requirements

整个 `rules` capability 废弃。规则内容（原 spec 4 个结构化 Requirement + 非结构化描述段）已迁移到 `CLAUDE.md` 的"规则（唯一事实来源）"章节。本次变更仅做文档组织调整，**规则本身内容未变**。

### Requirement: 代码符合命名规范

**Reason**: 规则内容已整合到 `CLAUDE.md` 的"务实编码原则 - 1. 命名即文档"和"项目特定规范 - 命名规范"章节。在 OpenSpec 文档体系中，编码规范属于项目入口（CLAUDE.md）职责，不属于 spec capability。

**Migration**:
- 查阅命名规范：`grep "命名即文档" CLAUDE.md` 或 `grep "项目特定规范" CLAUDE.md`
- 验证命令：`grep -c "is_\|has_\|can_" src/**/*.rs`（应匹配所有布尔变量）
- 旧 spec 文件：归档到 `openspec/changes/consolidate-rules-into-claude-md/archive/spec.md`

### Requirement: 函数单一职责

**Reason**: 规则内容已整合到 `CLAUDE.md` 的"Karpathy Guidelines - 2. Simplicity First"和"务实编码原则 - 2. 函数单一职责"。

**Migration**:
- 查阅：`grep "函数单一职责" CLAUDE.md`
- 验证：`wc -l src/storage/buffer_pool.rs | awk '{ if ($1 > 50) print "⚠️ 长函数" }'`
- 旧 spec 文件：见上

### Requirement: 测试覆盖核心逻辑

**Reason**: 规则内容已整合到 `CLAUDE.md` 的"Karpathy Guidelines - 4. Goal-Driven Execution"和"核心执行约束（8 条）- 4. 不测试通过不提交"。

**Migration**:
- 查阅：`grep "TDD Iron Law" CLAUDE.md`
- 验证：`cargo test 2>&1 | tail -5` 应显示 test result: ok
- 旧 spec 文件：见上

### Requirement: 需求完整性

**Reason**: 规则内容已整合到 `CLAUDE.md` 的"Karpathy Guidelines - 5. Requirements Integrity"和"核心执行约束（8 条）- 3. 不完整覆盖需求不实现"。

**Migration**:
- 查阅：`grep "Requirements Integrity" CLAUDE.md`
- 验证：每次功能提交前对照 Red Flags 第 6-7 条
- 旧 spec 文件：见上

---

## 非结构化文档段移除

原 `openspec/specs/rules/spec.md` 还包含以下非结构化段落，**全部内容已迁移到 CLAUDE.md**：

| 原段落 | 迁移到 CLAUDE.md 的章节 |
|--------|-----------------------|
| Karpathy Guidelines (5 条) | 规则 / 一、Karpathy Guidelines |
| 务实编码原则 (10 条铁律) | 规则 / 二、务实编码原则 |
| Workflow Designer (核心概念 + 6 条铁律) | 规则 / 三、Workflow Designer |
| 核心执行约束 (8 条) | 规则 / 四、核心执行约束 |
| 项目特定规范 (命名/结构/测试/提交) | 规则 / 五、项目特定规范（在五、技能执行规则之前/或合并到一~四） |
| 检查清单 (11 项) | 保留在 CLAUDE.md 原位置（"六、检查清单"） |
| Red Flags (11 条) | 保留在 CLAUDE.md 原位置（"Red Flags"） |

**回滚**：从本 change 的 `archive/spec.md` 恢复完整原文。
