## Context

**当前状态**（基于 M20 完成后）：
- `Value` 枚举（`src/executor/value.rs:45`）：`Int(i64)` / `String(String)` / `Null` / `Float(f64)` / `Bool(bool)` — 5 变体，`String` 内部堆分配
- `Expression::evaluate(&self, row: &[Value]) -> Result<Value>`（`src/executor/predicate.rs:25`）— 返回 owned Value
- `deserialize_tuple(data: &[u8], schema: &[ColumnType]) -> Result<Vec<Value>>`（`src/storage/page_format/tuple.rs:115`）— **第 160 行 `String::from_utf8(data[pos..pos + len].to_vec())` 每次 String 列做一次堆分配**
- 3 个 Scan 执行器（`ScanExecutor` / `IndexScanExecutor` / `IndexScanAllExecutor`）通过 `find_visible_version` / `read_tuple_from_data_page` 调 `deserialize_tuple`
- M20 完成（commit 4e17362）— `with_page_data` 闭包 API + 零拷贝页访问

**瓶颈**：
- 1K 行 × 1 String 列 × 平均 300B/单元 = 30万次 String 分配 + 30万次 拷贝
- 实际微基准（commit 4e17362 micro_bench）显示 read 路径 -2.46% 到 -8.33% 改进（来自 M20 `Vec<u8>` 消除），M36 进一步消除 String 分配应再提速 10-30%

**约束**：
- `Value` 不动（用户 B2 第 2 轮决策）— 不与 M37 (clone 消除) 重叠
- `Expression::evaluate()` 对外签名不变（用户 B2 第 3 轮决策）— 内部转调 `evaluate_ref().to_value()`
- 范围 = 3 个 Scan 执行器 + Expression trait（用户 B2 第 5 轮决策），不包含 Sort/Aggregate/Join
- 复用 M20 闭包 API 模式（用户 B3 选方案 A）— 与 `with_page_data` 同设计语言
- 不引入新 trait / 抽象层（M20 Metis 指令，沿用）
- 不能引入回归（cargo test 全部通过，micro/concurrent 套件 < 5% 波动）

## Goals / Non-Goals

**Goals**：
- 新增 `ValueRef<'a>` 零拷贝枚举，反序列化路径零 String 分配
- 复用现有 `deserialize_tuple` 二进制格式（5 tag bytes）
- `Expression::evaluate()` 内部转调 `evaluate_ref()`，**对外不破坏**（向后兼容）
- 3 个 Scan 执行器全部切换到零拷贝路径
- 验收：1K 行 String 分配 30万→0 OR micro_bench ≥ 15% 提速

**Non-Goals**：
- 不动 `Value` 枚举（`String` 保持 String 内部）— M37 范围
- 不动写路径（`UpdateExecutor` 写回需要 owned Value）— 反正要 `to_value()`
- 不改 Sort/Aggregate/Join（ValueRef 暂时不参与）— 后续里程碑扩展
- 不实现 M37 (clone 消除) / M29 (PG Extended Query) — 严格 M36 范围
- 不引入 `bytes::Bytes` / `Arc<[u8]>` 等新共享所有权原语（用户 B2 第 1 轮选 `&'a str`）

## Decisions

### 决策 1: `ValueRef<'a>` 零拷贝枚举

**选择**：在 `src/executor/value_ref.rs` 新增枚举：

```rust
/// 零拷贝 SQL 值视图，借用 'a 生命周期内的字节切片。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueRef<'a> {
    Int(i64),
    Text(&'a str),
    Null,
    Float(f64),
    Bool(bool),
}
```

**理由**：
- `Copy` — 24B（discriminant 8B + max payload 16B），栈复制零分配
- `Hash/Eq` — 后续 Sort/Aggregate/Join 零拷贝 key 可直接用（M36 暂不强制使用）
- `Debug` — 错误信息 + 调试日志
- 与 M20 `SlottedPageRef<'a>` 模式统一 — 零拷贝 view 用 `'a` 借用

**备选方案**：
- (A) `ValueRef::Text(Cow<'a, str>)` — Cow enum tag 内存 + 混淆所有调用点。否决
- (B) `ValueRef::Text(Arc<str>)` — 反序列化时一次 Arc 分配，共享所有权。但 M36 严格最小改动原则下，`&'a str` 更直接。否决
- ✅ (C) `&'a str` 原始借用 — 与 SlottedPageRef 一致，最直接

### 决策 2: `Expression::evaluate()` 内部转调 `evaluate_ref()`

**选择**：`Expression` trait 新增 `evaluate_ref`，`evaluate()` 内部调用 `evaluate_ref().to_value()`：

```rust
pub trait Expression: Send + Sync + Debug {
    /// 现有方法（向后兼容，内部转调）
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn Error + Send + Sync>>;

    /// M36 新增：零拷贝路径
    fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn Error + Send + Sync>>;
}
```

**`evaluate()` 默认实现**（用 trait 默认方法，3 个实现无需重复实现）：

```rust
pub trait Expression: Send + Sync + Debug {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let row_ref: Vec<ValueRef> = row.iter().map(Value::as_value_ref).collect();
        self.evaluate_ref(&row_ref).map(|vr| vr.to_value())
    }
    fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn Error + Send + Sync>>;
    // ... set_parameter_value
}
```

**理由**：
- 5 个 Expression 实现只需补 `evaluate_ref`，不破坏 `evaluate()` 调用方
- trait 默认方法（Rust 1.75+）— 避免 5 处实现重复 `to_value()` boilerplate
- 与 M20 `find_visible_version` "F 返回 Result<R>" 决策一致 — 让错误自然传播

**备选方案**：
- (A) 替换 `evaluate()` 为 `evaluate_ref()`，调用方改 — 与 M20 BREAKING 一致，但 209 处 Value 使用风险高
- (B) `evaluate()` 和 `evaluate_ref()` 独立实现 — 5 处重复实现 `to_value()` 模板代码。否决
- ✅ (C) `evaluate()` 用 trait 默认方法转调 `evaluate_ref().to_value()` — 单一来源 + 向后兼容

### 决策 3: `deserialize_value_refs` Vec 分配 + 零 String 分配

**选择**：复用 `deserialize_tuple` 二进制格式，TAG_STRING 用 `str::from_utf8` 借用不分配：

```rust
pub fn deserialize_value_refs(
    data: &'a [u8],
    schema: &[ColumnType],
) -> Result<Vec<ValueRef<'a>>> {
    let mut pos = 0;
    let mut values = Vec::with_capacity(schema.len());

    for _col_type in schema {
        let tag = data[pos]; pos += 1;
        match tag {
            TAG_INT => { /* ... i64::from_le_bytes(...) */ values.push(ValueRef::Int(n)); }
            TAG_STRING => {
                let len = u16::from_le_bytes([data[pos], data[pos+1]]) as usize;
                pos += 2;
                let s = std::str::from_utf8(&data[pos..pos+len])
                    .map_err(|e| StorageError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
                values.push(ValueRef::Text(s));  // 借用 data 内部
                pos += len;
            }
            // TAG_NULL / TAG_FLOAT / TAG_BOOL 同理
        }
    }
    Ok(values)
}
```

**Vec 分配规模**：1K 行 × 8B × schema.len() = 8KB（5 列 schema），相比原 `Vec<Value>` 8KB + 300KB String 分配，**总分配减少 ~97%**。

**理由**：
- 复用 5 tag bytes 格式 — 序列化格式不变，向后兼容
- Vec 分配是次要成本（相比 String 分配 300 倍差距）
- 借用 `&'a [u8]` 闭包内 scope — M20 `with_page_data` 模式直接套用

**备选方案**：
- (A) 写入调用方提供的 `&mut [ValueRef]` — 调用方预分配栈数组，零堆。否决（调用繁琐，且列数 schema 固定但调用方需感知）
- (B) 返回 `&[ValueRef<'a>]` 借用 `data` 整个 slice — 需要 data 对齐 + 生命周期严格，类似 Columnar store。否决（复杂度高）
- ✅ (C) `Result<Vec<ValueRef<'a>>>` — Vec 分配 8KB 可接受，最便利

### 决策 4: `ValueRef::to_value()` 是唯一 String 分配出口

**选择**：

```rust
impl<'a> ValueRef<'a> {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Int(n) => Value::Int(*n),  // 零分配
            Self::Text(s) => Value::String((*s).to_string()),  // 唯一堆分配
            Self::Null => Value::Null,  // 零分配
            Self::Float(f) => Value::Float(*f),  // 零分配
            Self::Bool(b) => Value::Bool(*b),  // 零分配
        }
    }
}
```

**理由**：
- 写路径 / 序列化 / 网络转 Value 必须 — 这是最小成本点
- Int/Float/Bool/Null 零分配
- 整个 M36 范围内 String 分配只在 `to_value()` 调用点

### 决策 5: 3 个 Scan 执行器闭包改造（统一模式）

**选择**：

```rust
// ScanExecutor / IndexScanExecutor / IndexScanAllExecutor — 有 snapshot 路径
let values_opt = self
    .buffer_pool
    .find_visible_version(row_id, snapshot, |bytes| {
        // M36: 零拷贝
        deserialize_value_refs(bytes, &self.schema)
            .map(|vrs| vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>())
    })
    .await?;

// 无 snapshot 路径
let values = read_tuple_from_data_page(
    &self.buffer_pool, row_id,
    |_vh, bytes| {
        deserialize_value_refs(bytes, &self.schema)
            .map(|vrs| vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>())
    },
).await?;
```

**理由**：
- 闭包内 `bytes: &[u8]` 借用 `with_page_data` scope
- `deserialize_value_refs` 返回的 `Vec<ValueRef<'_>>` 借用 bytes
- **整个借用链 0 分配**（Vec<ValueRef> 8KB 是次要成本）
- 与 M20 模式完全一致 — 改动最小

### 决策 6: 不改 UpdateExecutor / Sort / Aggregate / Join

**选择**：
- `UpdateExecutor` 写回需要 owned `Vec<Value>` 序列化 — 保持 `deserialize_tuple` 路径（M36 范围外）
- `Sort` / `Aggregate` / `Join` 当前接收 `Vec<Value>` — 保持现状（M36 范围外）
- 后续里程碑（M37 / Phase 3）扩展 ValueRef 到 Sort/Aggregate/Join

**理由**：
- 严格 M36 范围 = 3 个 Scan 执行器（用户 B2 第 5 轮决策）
- 避免跨越 M37 边界
- 写路径反正要 `to_value()`，受益有限

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| **`ValueRef<'a>` 借用逃逸** — 闭包外持有 ValueRef 指向已 drop 的 page data | 闭包内 `to_value()` 立即转 owned；类似 M20 L022，测试加 `cargo test -- --nocapture` |
| **`Expression::evaluate_ref` 3 个实现遗漏** — 编译失败 | trait 强制未实现编译错误；Task 5 集中添加 ColumnExpression / ConstantExpression / ParameterExpression |
| **Vec<ValueRef> 8KB 分配** — 比 String 分配小但仍存在 | 1K 行场景可接受；未来 Columnar store（M22）可彻底消除 |
| **`to_value()` 写回分配** — Scan 输出给上层仍有 String 分配 | 不可避免（上层要 owned Value）；M37 clone 消除可进一步优化 |
| **`str::from_utf8` 错误传播** — M20 不存在的新错误点 | 复用 `StorageError::Io(InvalidData)`，与现有 IO 错误路径一致 |
| **基准噪声掩盖收益** — 1K 行场景下 Vec 分配也是开销 | `--save-baseline before-m36` + `--baseline before-m36`；验收门槛看 String 分配数（30万→0）而非速度 |
| **`Expression::evaluate()` 默认方法性能** — trait 默认方法 + 闭包 `to_value()` 调用栈 | 默认方法用 `inline` 标注 + 编译器内联；微基准验证无回归 |
| **闭包内禁止 `.await` / 递归 BufferPool** | M20 已验证；M36 复用同模式，文档约束 + 代码 review |

## Migration Plan

**步骤**：
1. 新增 `src/executor/value_ref.rs` — `ValueRef<'a>` 枚举 + `to_value()` + `as_value_ref()` + 单元测试
2. 新增 `deserialize_value_refs` 在 `src/storage/page_format/tuple.rs` + 单元测试
3. 更新 `src/executor/predicate.rs` `Expression` trait — 新增 `evaluate_ref()` 默认方法 + 5 个实现补 `evaluate_ref`
4. 更新 `src/executor/{scan,index_scan,index_scan_all}.rs` — 闭包内改用 `deserialize_value_refs` + `to_value()`
5. 更新 `src/executor/mod.rs` / `src/storage/page_format/mod.rs` — 导出新类型
6. 跑全量 `cargo test` 验证 0 失败
7. 跑 `cargo clippy` 验证 0 warnings（M36 范围内）
8. 跑 `cargo bench --bench micro_bench -- --save-baseline before-m36` 留底
9. 跑 `cargo bench --bench micro_bench -- --baseline before-m36` 对比，确认 30万→0 分配或 ≥ 15% 提速
10. 跑 micro + concurrent 套件确认无回归
11. 提交 git commit（feat 风格）
12. `/opsx:archive m36-zero-copy-value-ref` 归档
13. 更新 `tasks.md` + `snapshot.md`

**回滚**：
- 单 commit revert 即可
- 不涉及数据迁移 / schema 变更
- `Expression::evaluate()` 对外签名不变，向后兼容
- 风险窗口：commit 到下一次 production 部署前

## Open Questions

- **Q1**: `ValueRef::Text` 是否要支持 `&'a [u8]` 变体（binary blob）？ — 当前 schema 无 Blob 类型，不需要。后续 M44+ 扩展 schema 时再加
- **Q2**: `Expression::evaluate_ref` trait 默认方法是否需要 `#[inline]` 标注？ — 微基准验证；M22/Phase 5 时统一优化
- **Q3**: 验收门槛是 30万→0 分配还是 ≥ 15% 速度？ — **双标准**：1K 行 String 分配 30万→0 **AND** micro_bench ≥ 5% 速度提升（M20 经验：单纯分配消除速度提升有限）
- **Q4**: `ValueRef` 是否需要 `From<&Value>` 转换？ — `Value::as_value_ref()` 足够；不引入额外 trait
- **Q5**: 5 个 Expression 实现的 `evaluate_ref` 闭包内是否可能 `.await`？ — 不可能（同步逻辑），与 M20 闭包一致
- **Q6**: 是否需要 `ValueRef<'_>` 反向转 `&[u8]` 用于 schema 验证？ — 不需要，schema 在反序列化时已传入

