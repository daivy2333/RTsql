# tasks: ARC-202609241843a improvements 预批准条目归档

> 用户触发：2026-09-24（openspec-archivist 全量审计 + Gate 1 批准实施与 skill 调用豁免）；预批准依据：2026-09-12 用户裁定（条目自注待物理归档）。

## 执行清单

- [x] 1. 记录源文档 mtime（improvements 2026-09-24 18:14:40）
- [x] 2. 预检：`openspec validate --all` 40 passed / 0 failed；无活跃 change
- [x] 3. 生成 ARC ID：ARC-202609241843a
- [x] 4. 创建 proposal / archive / specs/cleanup-batch / tasks
- [x] 5. I018/I027 完整原文写入 carrier（脚本逐字提取）
- [ ] 6. 验证 carrier change（openspec validate）
- [ ] 7. OpenSpec 集成归档 carrier（--skip-specs）
- [ ] 8. 复查源文档 mtime 未被并发修改
- [ ] 9. 源文档精准移除 I018/I027
- [ ] 10. 源文档追加 `<!-- arc: ARC-202609241843a -->` 墓碑
- [ ] 11. Gate 2 验证：validate 复跑、源文档结构完整、执行报告
