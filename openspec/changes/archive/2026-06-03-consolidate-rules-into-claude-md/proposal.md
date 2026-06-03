## Why

当前 OpenSpec 文档体系中，`openspec/specs/rules/spec.md`（266 行）作为编码规范的"唯一事实来源"，而 `CLAUDE.md` 仅承担文档索引入口。这种双层结构导致三个问题：

1. **双份维护成本**：任何规则变更需要在两处同步（CLAUDE.md 的"快速开始" + rules spec）
2. **CLAUDE.md 规则深度不足**：当前只有"检查清单 + Red Flags"，缺少 Karpathy Guidelines / 务实编码原则 / Workflow Designer / 核心执行约束 / 技能执行规则五大体系的完整描述
3. **上下文加载冗余**：agent 读取 rules spec 才能获得完整规则，索引入口不能自包含

本次变更将规则全文整合到 `CLAUDE.md`，废弃 `openspec/specs/rules/`，让 `CLAUDE.md` 升级为"文档索引 + 规则唯一事实来源"双角色。新增 [ADR-009](#adr-009) 记录此次架构决策。

## What Changes

- **REMOVED**：删除 `openspec/specs/rules/spec.md`（266 行规则内容），废弃 `openspec/specs/rules/` 目录
- **MODIFIED**：扩展 `CLAUDE.md`（从 ~108 行扩展到 ~350+ 行），新增 5 大规则章节：
  - 一、Karpathy Guidelines（行为约束）
  - 二、务实编码原则（代码质量 10 条铁律）
  - 三、Workflow Designer（流程框架）
  - 四、核心执行约束（8 条铁律）
  - 五、技能执行规则（强制）
- **MODIFIED**：删除 `CLAUDE.md` 文档体系表中的 `rules/` 行
- **MODIFIED**：`.claude/docs/snapshot.md` 新增"文档体系 2.0 升级"段落
- **UNCHANGED**：`openspec/specs/{architecture,learned,optimization,references}/` 4 个 spec 不动
- **UNCHANGED**：`.claude/docs/{tasks.md,archive.md,superpowers/}` 不动
- **UNCHANGED**：`openspec/config.yaml` 不动

## Capabilities

### New Capabilities

无。`CLAUDE.md` 是项目入口文档（Agent 索引），不属于 OpenSpec spec capability。

### Modified Capabilities

- `rules`: **REMOVED**。规则内容迁移到 `CLAUDE.md`，原 `openspec/specs/rules/spec.md` 删除。本变更通过 `specs/rules/spec.md` 的 REMOVED delta 表达。
  - **影响范围**：仅文档组织变更，不改变任何规则本身的内容
  - **变更原因**：解决"规则双份维护"和"CLAUDE.md 索引深度不足"
  - **回滚方案**：从 git 历史恢复 `openspec/specs/rules/spec.md` 即可

## Impact

| 影响对象 | 影响范围 | 风险 |
|---------|---------|------|
| `CLAUDE.md` | 大幅扩展 | 需保留旧版本作为基础 |
| `openspec/specs/rules/` | 整个目录删除 | 需先备份内容到 CLAUDE.md |
| `openspec/specs/{architecture,learned,optimization,references}/` | 不动 | 0 |
| `.claude/docs/snapshot.md` | 追加段落 | 低 |
| `openspec/changes/` | 新增本提案 | 0 |
| `.codegraph/` 索引 | 后续可能需要重建 | 低（不阻塞） |
| 外部引用 | 无（rules spec 是项目内部约定） | 0 |
| 已有 commits | `bca4785` 等保留 | 0 |

**关联 ADR**：
- 新增 [ADR-009：规则文档化位置从 specs/rules 迁移到 CLAUDE.md]
- 引用 [ADR-001]~[ADR-008] 中提到"约束/规则"的段落（如 ADR-005 务实 Clippy 策略）— 实际只引用规范文本不需修改 ADR 本身

**回滚方案**：
```bash
# 1. 从 git 恢复 rules/spec.md
git checkout HEAD~1 -- openspec/specs/rules/spec.md

# 2. 恢复 CLAUDE.md 到旧版本
git checkout HEAD~1 -- CLAUDE.md

# 3. 重新生成 rules 目录
# （如需要可重新跑 openspec-init 的旧版流程）

# 4. 删除本 change
openspec archive consolidate-rules-into-claude-md --skip-spec-sync
```

**不做什么**：
- 不重写其他 4 个 spec（architecture/learned/optimization/references）
- 不修改 `openspec/config.yaml`
- 不重新初始化 `.codegraph/` 索引（变更后 agent 可选重建）
- 不把代码规范（如"snake_case 命名"）拆分到 `references/` 中
- 不修改 `tasks.md`（没有任务进度变化）
