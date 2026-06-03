## Context

**当前状态**（2026-06-02 已初始化的 OpenSpec 文档体系）：
- `openspec/specs/rules/spec.md`（266 行）— 编码规范"唯一事实来源"
- `CLAUDE.md`（~108 行）— 仅做文档索引入口
- 双层结构导致维护成本高、CLAUDE.md 索引深度不足

**约束**：
- Rust 项目，产出物必须简体中文
- openspec spec 必须使用 Given/When/Then + Purpose/Requirements/Scenario 标准格式
- OpenSpec 验证器要求 spec 内容格式合规
- 已存在 `.codegraph/` 索引（基于当前文件状态）

**利益相关方**：
- Agent / AI 编码助手 — 主要消费者，需要完整规则才能正确行为
- 开发者 — 维护者，关心变更成本和向后兼容
- 团队新成员 — 间接消费者

## Goals / Non-Goals

**Goals**：
1. `CLAUDE.md` 成为自包含的项目入口：单文件加载即可获得全部规则
2. 消除双份维护：规则只在 `CLAUDE.md` 维护
3. 5 大规则体系完整呈现：Karpathy / 务实编码 / Workflow / 核心约束 / 技能执行
4. 保持 OpenSpec 验证通过：废弃 `rules` capability 不破坏其他 4 个 spec
5. 回滚可行：单次 commit 范围内可逆

**Non-Goals**：
1. 不重写其他 4 个 spec 的内容
2. 不修改 `openspec/config.yaml` 的 schema/rules 字段
3. 不重命名或重组其他 spec 目录
4. 不为本次变更重新初始化 `.codegraph/` 索引
5. 不修改 `tasks.md`（无功能任务进度变化）

## Decisions

### 决策 1：CLAUDE.md 采用 5 章节规则结构

**选择**：在 CLAUDE.md 末尾追加 5 大规则章节（一~五），原有"项目概览/文档体系/读取顺序/检查清单/Red Flags" 全部保留为前导。

**理由**：
- 向后兼容：旧内容完整保留，新规则章节在末尾追加，agent 加载即可获得全量信息
- 渐进可读：先看索引（轻量），再看规则（深度），符合阅读心智模型
- 不破坏链接：所有 `openspec/specs/xxx/` 引用保持有效

**替代方案**：
- **A. 规则放在文件最前**（"规则优先"）→ 缺点：违反索引入口定位，新人/agent 先看规则再看文档体系，认知负担重
- **B. 拆成 CLAUDE.md（索引）+ RULES.md（规则）两个文件** → 缺点：仍是双份维护，违背"单文件自包含"目标
- **C. 规则放在子目录 `.claude/rules.md`** → 缺点：需要 agent 知道去读 `.claude/`，破坏"CLAUDE.md 是入口"的简洁性

### 决策 2：废弃 rules 目录的物理处理

**选择**：执行 `rm -rf openspec/specs/rules/`，不保留空目录。

**理由**：
- OpenSpec 验证器对空目录不友好（不报错但产生噪声）
- 防止误用：保留空目录会让人误以为"未来还会放回内容"
- 简洁：明确表达"已废弃"语义

**替代方案**：
- **A. 保留目录 + 放 README 提示** → 缺点：长期遗留垃圾文档
- **B. 软链接到一个 README 标记已废弃** → 缺点：增加维护成本，软链接在 git 跨平台不友好
- **C. 改名 `rules-archived/` 备份** → 优点：可回滚时直接恢复  → **接受！同时执行**：删除前先把当前内容 git 备份到 openspec/changes/.../archive/，并在 proposal.md 引用其路径

**最终选择**：删除 rules/ 目录，但通过 git 历史 + 本 change 的 archive/ 子目录双重保障回滚能力。

### 决策 3：specs delta 表达

**选择**：在 `openspec/changes/consolidate-rules-into-claude-md/specs/rules/spec.md` 写 REMOVED delta，列出被删除的 Requirements（指向原 rules spec 章节编号）。

**理由**：
- 符合 OpenSpec Delta Specs 约定
- 后续 `openspec archive --sync-specs` 时，OpenSpec 自动从主 specs 中删除 rules
- 保留审计轨迹：归档后能从 git 历史看到"曾经存在过 rules spec"

**替代方案**：
- **A. 不写 delta，直接物理删除** → 缺点：违反 OpenSpec 工作流，archive 时同步状态错乱
- **B. 写 MODIFIED delta 标记"已迁移"** → 缺点：MODIFIED 表达"内容变更"，而我们实际是"整个 capability 移除"

### 决策 4：CLAUDE.md 行数控制

**选择**：目标 ~350 行（5 规则章节 + 原有内容），不上限。

**理由**：
- 5 大规则体系完整内容需要约 200-250 行
- 原有内容（索引 + 检查清单 + Red Flags）约 110 行
- 总和 ~350 行，单文件可读，仍属于"加载成本可接受"范围

**替代方案**：
- **A. 拆分到子文件 + 链接** → 缺点：破坏单文件自包含目标
- **B. 极简压缩到 200 行** → 缺点：规则描述不完整，违背新规范
- **C. 上不封顶** → 接受：允许扩展到 500 行内

## Risks / Trade-offs

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| **CLAUDE.md 行数膨胀到 500+ 行** | 单次加载成本上升 | 控制规则描述简洁度，避免冗余 |
| **规则描述与 openspec-init v2 模板不完全一致** | 新 skill 调用时可能产生"模板漂移" | 在变更完成后比对 v2 模板，逐项对齐 |
| **archive 时 sync-specs 误删其他 spec 内容** | 高 | 先在 dry-run / --no-sync 模式下验证，确认无误后再 sync |
| **git 历史找不到旧 rules spec** | 回滚失败 | 保留 openspec/changes/.../archive/spec.md 镜像，引用其路径 |
| **`.codegraph/` 索引过时** | agent 用 codegraph 查 "rules" 时找不到 | 接受：变更后由用户决定是否重建索引 |
| **新会话加载 CLAUDE.md 时上下文占用增加** | 中等 | 5 大规则章节用紧凑结构，避免冗长案例 |
| **"5 大规则体系"是 openspec-init v2 的术语** | 团队/agent 可能不熟悉 | 在规则章节开头说明来源和含义 |

## Migration Plan

### 执行步骤（顺序敏感）

```
Phase A：备份
  1. cp openspec/specs/rules/spec.md openspec/changes/consolidate-rules-into-claude-md/archive/spec.md
     （创建 mirror 副本，确保回滚可行）

Phase B：写新 CLAUDE.md
  2. 读取 openspec/specs/rules/spec.md 完整 266 行内容
  3. 拼接 = 原 CLAUDE.md 108 行（保留）+ 5 大规则章节（约 250 行）
  4. Write 完整新 CLAUDE.md 到 /home/daivy/projects/RTsql/CLAUDE.md

Phase C：物理删除
  5. rm -rf openspec/specs/rules/
  6. 验证 openspec list spec 不再包含 rules

Phase D：状态同步
  7. Edit .claude/docs/snapshot.md，在"文档体系变更"表格新增"v2.0 升级"行
  8. Edit .claude/docs/snapshot.md，删除"OpenSpec v1.4.0 初始化"过时描述

Phase E：OpenSpec 验证
  9. openspec validate --specs（确认 4 个 spec 通过）
  10. openspec list spec（确认 4 个）
```

### 部署策略

- 单一 commit：包含 CLAUDE.md 修改 + rules/ 删除 + snapshot.md 更新
- commit message 格式：`docs(consolidate-rules): 整合规则到 CLAUDE.md，废弃 rules/`
- 推送前本地验证：跑 `openspec validate --specs` 必须通过

### 回滚策略

```bash
# 1. 从 mirror 恢复 rules
mkdir -p openspec/specs/rules
cp openspec/changes/consolidate-rules-into-claude-md/archive/spec.md openspec/specs/rules/spec.md

# 2. 恢复 CLAUDE.md
git checkout HEAD~1 -- CLAUDE.md

# 3. 恢复 snapshot.md（如需要）
git checkout HEAD~1 -- .claude/docs/snapshot.md

# 4. 归档本 change（跳过 spec 同步）
openspec archive consolidate-rules-into-claude-md --skip-spec-sync
```

## Open Questions

无。本次变更的范围和策略已通过 AskUserQuestion 确认（4 个问题全部分别明确选择），执行细节无歧义。
