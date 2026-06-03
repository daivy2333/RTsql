# M36: 零拷贝 ValueRef — 任务清单

> 最后更新：2026-06-03（brainstorming 完成，方案 A 整合）

## T1: `ValueRef<'a>` 枚举 + 单元测试

- [ ] 在 `src/executor/value_ref.rs` 新建 `ValueRef<'a>` 枚举（`Int / Text / Null / Float / Bool`）
- [ ] 实现 `Copy / Clone / Debug / PartialEq / Eq / Hash`
- [ ] 实现 `to_value()` 方法（Text 唯一堆分配）
- [ ] 实现 `is_null()` / `as_float()` / `as_bool()` / `equals()` / `gt()` / `lt()` / `ge()` / `le()` 等适配 Value 的方法
- [ ] 新增 `Value::as_value_ref()` 方法（owned → borrowed view）
- [ ] 单元测试：to_value 分配行为 / as_value_ref 借用 / Hash/Eq / size_of
- 验证：`cargo test --lib value_ref` 全部通过

## T2: `deserialize_value_refs` 实现 + 单元测试

- [ ] 在 `src/storage/page_format/tuple.rs` 新增 `deserialize_value_refs(data: &'a [u8], schema) -> Result<Vec<ValueRef<'a>>>`
- [ ] 复用 5 tag bytes 格式，TAG_STRING 用 `std::str::from_utf8` 借用不分配
- [ ] 错误处理：truncated / unknown tag / invalid UTF-8 都转 `StorageError::Io`
- [ ] 单元测试：5 列 roundtrip / 借用语义验证（`ptr >= data.as_ptr()`） / 错误路径
- [ ] 关键测试：`deserialize_value_refs_alloc_count` — 用 `AllocationCounter` 验证 String 列 0 分配
- 验证：`cargo test --lib tuple` 全部通过

## T3: `Expression` trait 扩展 + 5 个实现补 `evaluate_ref`

- [ ] 在 `src/executor/predicate.rs` `Expression` trait 新增 `fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, _>`
- [ ] `evaluate()` 改为 trait 默认方法：转 `Vec<ValueRef>` via `as_value_ref` + 调 `evaluate_ref().to_value()`
- [ ] `ColumnExpression::evaluate_ref` — 借用 `row[self.column_index]`
- [ ] `ConstantExpression::evaluate_ref` — 返回 `self.value.as_value_ref()`
- [ ] `ParameterExpression::evaluate_ref` — 返回 `self.value.lock().as_value_ref()`
- 验证：`cargo check --lib` 全部 Expression 编译通过

## T4: 3 个 Scan 执行器闭包改造

- [ ] `ScanExecutor::next` 有 snapshot 路径：`find_visible_version` 闭包内 `deserialize_value_refs` + `to_value()`
- [ ] `ScanExecutor::next` 无 snapshot 路径：`read_tuple_from_data_page` 闭包内同样改造
- [ ] `IndexScanExecutor::next` — 同上两条路径
- [ ] `IndexScanAllExecutor::next` — 同上两条路径
- 验证：`cargo test --test executor_test` 全部通过

## T5: 模块导出

- [ ] `src/executor/mod.rs` 加 `pub use value_ref::ValueRef;`
- [ ] `src/storage/page_format/mod.rs` 加 `pub use tuple::deserialize_value_refs;`
- 验证：`cargo check --lib` 0 errors

## T6: 测试 + Lint + 格式化

- [ ] 跑全量 `cargo test --lib --tests` 验证 0 失败
- [ ] 跑 `cargo clippy --all-targets` 验证 M36 范围内 0 warnings
- [ ] 跑 `cargo fmt -p rtsql` + `rustfmt` 所有 M36 改动文件
- [ ] 新增 2 个集成测试：`tests/executor_test.rs` 内 `M36_zero_copy_string_borrow` + `M36_value_ref_via_value`
- 验证：全量测试通过 + clippy 通过 + fmt 干净

## T7: 性能验证

- [ ] `cargo bench --bench micro_bench -- --save-baseline before-m36`（M20 m20-after 作起点）
- [ ] 实施完成后 `cargo bench --bench micro_bench -- --baseline before-m36` 对比
- [ ] 验收门槛（双标准）：1K 行 String 分配 30万→0 **AND** micro_bench ≥ 5% 速度提升
- [ ] 跑 concurrent_bench 确认无回归
- [ ] 记录到 `learned/spec.md` L025（M36 实测性能）

## T8: 文档同步 + 归档

- [ ] 更新 `.claude/docs/tasks.md` M36 状态 + 实际性能数据
- [ ] 更新 `.claude/docs/snapshot.md` M36 完成
- [ ] 写 `learned/spec.md` L025（M36 实测性能数据 + 与设计目标对比）
- [ ] `/opsx:archive m36-zero-copy-value-ref`
- [ ] 同步增量 spec 到 main specs
- [ ] git commit（feat 风格）— 单一 commit 包含全部 M36 改动

## T9: 边界保护（可选）

- [ ] `ValueRef` 是否需要 `Send + Sync` 验证 — `&'a str` 自动 Send + Sync（`str` 是）
- [ ] `Expression` 闭包 `evaluate_ref` 文档加 SAFETY 警告（不要 .await / 递归 BufferPool）
- [ ] 反 cargo test hang 防御（`cargo test -- --nocapture`）
