# AGENTS.md — 跨平台入口适配器

> 此文件为 Codex 和 OpenCode 加载 OpenSpec 体系规则的入口。
> **不**复制公共规则；所有规则以 `CLAUDE.md` 为唯一来源。

## OpenSpec workflow rules

Before planning, implementing, reviewing, or updating project documentation:

1. Read `CLAUDE.md` completely.
2. Treat its OpenSpec roles, Gates, BDD, TDD, verification, and editing rules as mandatory.
3. Load only the skill references required by the active task.
4. Do not copy those rules into this file; `CLAUDE.md` is their single source.
5. When the platform automatically resumes pending work, re-check the nearest authorization, capability, or stop boundary before continuing.

## 项目身份

RTsql — 异步协程驱动的高性能嵌入式关系型数据库。详细项目描述见 `.claude/docs/SNAPSHOT.md`。

## 加载入口

- Claude Code: 使用 `.claude/skills/` 和本仓库的 OpenSpec skill 入口
- Codex / OpenCode: 使用 `.agents/skills/`（见 Phase 7 安装结果）
- 公共规则统一从 `CLAUDE.md` 加载
