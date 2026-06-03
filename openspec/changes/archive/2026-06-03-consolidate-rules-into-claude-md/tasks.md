## 1. 备份与准备

- [ ] 1.1 备份当前 `openspec/specs/rules/spec.md` 到本 change 的 `archive/spec.md`，作为回滚镜像
  - **验收**：`diff openspec/specs/rules/spec.md openspec/changes/consolidate-rules-into-claude-md/archive/spec.md` 无输出
  - **依赖**：无
- [ ] 1.2 备份当前 `CLAUDE.md` 到 `archive/CLAUDE.md.before`
  - **验收**：`diff CLAUDE.md archive/CLAUDE.md.before` 无输出
  - **依赖**：无
- [ ] 1.3 验证当前 OpenSpec 状态
  - **命令**：`openspec list spec`（期望 5 个），`openspec validate --specs`（期望通过）
  - **依赖**：1.1, 1.2

## 2. 扩展 CLAUDE.md（核心变更）

- [ ] 2.1 在 CLAUDE.md 末尾追加"规则（唯一事实来源）"大章节
  - **包含**：一、Karpathy Guidelines；二、务实编码原则；三、Workflow Designer；四、核心执行约束；五、技能执行规则
  - **位置**：原"## Red Flags"之前或之后（保持"检查清单"和"Red Flags"在末尾）
  - **依赖**：1.2
- [ ] 2.2 删除 CLAUDE.md "文档体系"表中的 `rules/` 行
  - **原因**：rules spec 已废弃
  - **依赖**：2.1
- [ ] 2.3 更新 CLAUDE.md "读取顺序"表：删除"读 rules spec"指令
  - **依赖**：2.1
- [ ] 2.4 更新 CLAUDE.md "快速开始"段落：删除"阅读 rules spec"指令
  - **依赖**：2.1
- [ ] 2.5 验证 CLAUDE.md 行数：目标 ~350 行（不超过 500）
  - **命令**：`wc -l CLAUDE.md`
  - **依赖**：2.1-2.4

## 3. 物理删除 rules 目录

- [ ] 3.1 确认 archive/spec.md 备份完成（依赖 1.1）
- [ ] 3.2 删除 `openspec/specs/rules/` 目录
  - **命令**：`rm -rf openspec/specs/rules/`
  - **验证**：`ls openspec/specs/` 应只剩 4 个目录
  - **依赖**：3.1
- [ ] 3.3 OpenSpec 验证
  - **命令**：`openspec list spec`（期望 4 个），`openspec validate --specs`（期望通过）
  - **依赖**：3.2

## 4. 状态文档同步

- [ ] 4.1 更新 `.claude/docs/snapshot.md`：在"文档体系变更"表格新增"v2.0 升级（2026-06-03）"行
  - **内容**：本次升级要点
  - **依赖**：3.3
- [ ] 4.2 更新 `.claude/docs/snapshot.md` 顶部"最后更新"日期
  - **依赖**：4.1
- [ ] 4.3 验证 `.codegraph/` 索引状态（可选）
  - **命令**：`codegraph status`（如有），如 unhealthy 则提示用户重建
  - **依赖**：4.2
  - **不阻塞**：本次变更不强制重建索引

## 5. 端到端验证

- [ ] 5.1 OpenSpec 完整验证
  - **命令**：`openspec validate --specs --strict`（如支持）
  - **期望**：4 个 spec 全部通过
- [ ] 5.2 文件结构检查
  - **命令**：
    ```bash
    test ! -d openspec/specs/rules && echo "✅ rules/ 已删除"
    test -f CLAUDE.md && echo "✅ CLAUDE.md 存在"
    test -f openspec/changes/consolidate-rules-into-claude-md/archive/spec.md && echo "✅ 备份存在"
    wc -l CLAUDE.md  # 行数检查
    ```
- [ ] 5.3 CLAUDE.md 内容自检
  - **命令**：
    ```bash
    grep -c "## 一、Karpathy" CLAUDE.md   # 应为 1
    grep -c "## 五、技能执行规则" CLAUDE.md  # 应为 1
    grep -c "## Red Flags" CLAUDE.md      # 应为 1（保留原 Red Flags）
    ```
- [ ] 5.4 git 状态检查
  - **命令**：`git status` 应显示：
    - `modified: CLAUDE.md`
    - `modified: .claude/docs/snapshot.md`
    - `deleted: openspec/specs/rules/`
    - `untracked: openspec/changes/consolidate-rules-into-claude-md/`
  - **依赖**：5.1-5.3

## 6. 归档（最后一步）

- [ ] 6.1 执行 OpenSpec 归档
  - **命令**：`openspec archive consolidate-rules-into-claude-md`
  - **期望**：rules spec 从主 specs 移除，change 移入 `openspec/changes/archive/`
- [ ] 6.2 提交变更
  - **命令**：`git add -A && git commit -m "docs(consolidate-rules): 整合规则到 CLAUDE.md，废弃 rules/"`
  - **依赖**：6.1

---

## 关联里程碑

本次变更**不属于 M19-M48 性能优化路线**，属于**文档体系基础设施重构**。无 milestone 编号关联。

## 回滚策略（如执行中失败）

按 `design.md` Migration Plan 的"回滚策略"段落执行：
1. 从 `archive/spec.md` 恢复 rules spec
2. `git checkout HEAD~1 -- CLAUDE.md` 恢复 CLAUDE.md
3. 重新生成 rules 目录
4. 删除本 change

详见 `openspec/changes/consolidate-rules-into-claude-md/design.md` 第 "回滚策略" 段。
