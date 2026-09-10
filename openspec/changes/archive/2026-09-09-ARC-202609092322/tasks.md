# tasks: ARC-202609092322 文档体系生命周期清理

> 用户触发：2026-09-09（openspec-archivist 显式调用）；Gate 1 批准：2026-09-09（"批准"，含 .claude/skills 删除、I018/I021/I027 Stale-Warn、R05 归档三项判定）。
> 依据：本会话 Explorer 两轮调查（代码现场 + 文档体系盘点）+ 判定规则 + carrier 协议。

## 执行清单

- [x] 1. 记录源文档 mtime（improvements 21:52:07 / references 14:06:51）
- [x] 2. 预检：`openspec validate --all` 19 passed / 0 failed；无活跃 change
- [x] 3. 生成 ARC ID：ARC-202609092322
- [x] 4. 创建 proposal / specs/improvements / specs/references / tasks
- [x] 5. Archive 条目完整原文写入 carrier（7 条 I + R05，awk 逐字节提取）
- [ ] 6. 验证 carrier change
- [ ] 7. OpenSpec 集成归档 carrier（--skip-specs，归档文本非 delta spec）
- [ ] 8. 复查源文档 mtime 未被并发修改
- [ ] 9. 源文档精准移除 7 条 I 条目 + R05
- [ ] 10. 源文档追加 `<!-- arc: -->` 墓碑（improvements 7 条 / references 1 条）
- [ ] 11. I018/I021/I027 落 ⚠️ STALE 标记
- [ ] 12. Delete：.claude/commands/opsx/ 与 .claude/skills/ 5 个旧 skill（清空后移除空目录）
- [ ] 13. Gate 2 验证：validate 复跑、源文档结构完整、无活跃引用残留、执行报告
