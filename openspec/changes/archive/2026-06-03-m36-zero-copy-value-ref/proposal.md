## Why

`Expression::evaluate()` 返回 `Value` 枚举，其中 `Value::String(String)` 每次反序列化都要堆分配。`deserialize_tuple`（`src/storage/page_format/tuple.rs:115`）在第 160 行 `String::from_utf8(data[pos..pos + len].to_vec())` 每次 String 列都做一次堆分配，1K 行 × 1 String 列 × 平均 300B = 30万次分配。Phase 1 基础设施（M41/M30/M38）和 M20 零拷贝 SlottedPageRef（commit 4e17362）已全部完成，M20 打通了"页 → tuple bytes"链路，M36 继续打通"tuple bytes → Value 枚举"，彻底消除读路径 String 分配。

## What Changes

- **新增 `ValueRef<'a>` 零拷贝枚举**（`src/executor/value_ref.rs`）：`Int(i64)` / `Text(&'a str)` / `Null` / `Float(f64)` / `Bool(bool)`，实现 `Copy/Hash/Eq/Debug`
- **新增 `ValueRef::to_value()`** — 转换为 owned `Value`，唯一 String 分配出口
- **新增 `Value::as_value_ref()`** — owned Value 借用 view，`String(s) -> Text(s.as_str())` 借用 s 内部
- **新增 `deserialize_value_refs(data: &'a [u8], schema) -> Result<Vec<ValueRef<'a>>>`**（`src/storage/page_format/tuple.rs`）— 复用 `deserialize_tuple` 的二进制格式，TAG_STRING 用 `str::from_utf8` 借用不分配
- **`Expression` trait 新增 `evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>>`**，所有实现（ColumnExpression / ConstantExpression / ParameterExpression）需补
- **`Expression::evaluate()` 内部转调** `evaluate_ref().to_value()`，**对外不破坏**
- **3 个 Scan 执行器改造**：`find_visible_version` / `read_tuple_from_data_page` 闭包内改用 `deserialize_value_refs` + `.to_value()`（与 M20 模式一致）
- **新增零拷贝分配计数基准**：在 `benches/` 套件中对比改前改后 1K 行 String 分配数，验收门槛（双标准 AND）：1K 行 String 分配 30万→0 **AND** micro_bench ≥ 5% 速度提升

## Capabilities

### New Capabilities
- `zero-copy-value-ref`: 反序列化路径的零拷贝 Value view（`ValueRef<'a>` + `deserialize_value_refs` + `Expression::evaluate_ref`）

### Modified Capabilities
（无现有 spec 涵盖此能力，全部为新增）

## Impact

**影响代码**（按行数估算）：
- `src/executor/value_ref.rs`: 新文件 +120 行（ValueRef 枚举 + 方法 + 测试）
- `src/storage/page_format/tuple.rs`: 新增 `deserialize_value_refs` +60 行
- `src/executor/predicate.rs`: `Expression` trait 扩展 + 实现 evaluate_ref（3 个实现 × 10 行）
- `src/executor/{scan,index_scan,index_scan_all}.rs`: 各 -10/+20 行（闭包改造）
- `src/executor/mod.rs`: 导出 ValueRef +1 行
- `src/storage/page_format/mod.rs`: 导出 deserialize_value_refs +1 行

**影响测试**（必须保持全绿）：
- `src/storage/page_format/tuple.rs` 测试段：新增 5 个 ValueRef 单元测试
- `src/executor/value_ref.rs` 测试段：新增 4 个 ValueRef 单元测试
- `tests/executor_test.rs`: 新增 2 个集成测试（M36_zero_copy_string_borrow + M36_value_ref_via_value）

**性能影响**：
- 读路径 1K 行 × 1 String 列 30万 String 分配 → 0
- Vec<ValueRef> 8B × schema.len() 分配（1K 行 × 8B = 8KB，替代原 Vec<Value> 8KB + 300KB String）
- Scan 路径整体 -10% 到 -30%（取决于 String 列占比）
- `to_value()` 一次性分配（写回需要）

**风险**：
- `ValueRef<'a>` 借用 'a 生命周期 — 不能跨 await（M20 已验证可行，复用同模式）
- `Expression::evaluate_ref` 需补全 5 个实现 — 编译期强制
- 闭包内禁止 `.await` / 递归调用 BufferPool（M20 经验，文档约束）

**回滚方案**：
- 全部变更在单一 git commit 内
- 通过 `git revert <commit>` 即可回滚
- 不涉及数据迁移 / schema 变更 / 外部 API 破坏
- `Expression::evaluate()` 对外签名不变，向后兼容

**相关 ADR**：
- 引用 M20 决策 1（`with_page_data` 闭包 API）+ L022 记录（E0505 / unsafe hang 3 次失败）
- 引用 M20 `learned/spec.md` L024（微基准 -2.46% 到 -8.33%，M36 应进一步消除分配）
