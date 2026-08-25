# Migration Carrier ARCHIVED

- **Status**: archived
- **归档日期**: 2026-08-25
- **执行**: openspec-init
- **覆盖**: 100%（86 semantic entries = 86 mapped = 86 verified, unmapped = 0, skipped = 0）
- **内容**: 本 carrier 包含迁移前所有活动经验源的完整原文 + 覆盖清单（COVERAGE.md）
- **归档后**: 已移除活动副本（参见 `openspec/changes/` 旧活动 spec 目录的删除记录）
- **不可变**: 本 carrier 全文不得修改；恢复时使用 `COVERAGE.md` 的编号映射

## 路径

- 覆盖清单: `COVERAGE.md`
- 旧源全文: `sources/*.md`（10 个文件）
- 旧源列表: architecture, learned, optimization, references, buffer-pool-concurrency, data-scan-path, tx-id-allocation-benchmark, zero-copy-page-access, zero-copy-value-ref, archive.md

## 恢复入口

1. 读取 `COVERAGE.md` 找编号映射
2. 读取 `sources/<file>.md` 找旧文件全文
3. 跳转新 spec 文件查找对应 Mxx/Dxx/Kxx/Rxx/Ixx 条目
