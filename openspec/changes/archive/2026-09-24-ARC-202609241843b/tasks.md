# tasks: ARC-202609241843b 文档体系退役清理

> 用户触发：2026-09-24（openspec-archivist 全量审计）；Gate 1 批准：2026-09-24（「给出豁免，允许调用需要的skill，批准实施」）。

## 执行清单

- [x] 1. 记录源文档 mtime（project-model 2026-09-10 15:15 / decisions 2026-08-25 15:12 / knowledge 2026-09-08 19:41）
- [x] 2. 预检：`openspec validate --all` 40 passed / 0 failed；无活跃 change
- [x] 3. 生成 ARC ID：ARC-202609241843b（a 为 I018/I027 预批准批次）
- [x] 4. Merge 前置写入：project-model M17/M18/M06 行、runbook R27、analysis R28、references R27/R28
- [x] 5. Archive 条目完整原文写入 carrier（脚本逐字提取：M07/M16、D01-D12、K01-K38 含原分节）
- [ ] 6. 验证 carrier change（openspec validate）
- [ ] 7. OpenSpec 集成归档 carrier（--skip-specs）
- [ ] 8. 复查源文档 mtime 未被并发修改
- [ ] 9. 源文档精准移除 M07/M16；decisions/knowledge 置为退役空壳
- [ ] 10. 源文档追加 `<!-- arc: ARC-202609241843b -->` 墓碑（project-model 2 条 / decisions 12 条 / knowledge 38 条）
- [ ] 11. Gate 2 验证：validate 复跑、源文档结构完整、执行报告
