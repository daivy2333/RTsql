# zero-copy-value-ref 规范

> 版本: 1.0  
> 最后更新: 2026-06-03

## ADDED Requirements

### Requirement: 数据页 tuple 反序列化 MUST 提供零拷贝路径

`deserialize_value_refs(data: &'a [u8], schema: &[ColumnType]) -> Result<Vec<ValueRef<'a>>>` MUST 提供零拷贝反序列化 API，复用现有 TAG_INT / TAG_STRING / TAG_NULL / TAG_FLOAT / TAG_BOOL 5 tag bytes 格式，TAG_STRING 路径 MUST NOT 分配 `String`（使用 `std::str::from_utf8` 借用 data 内部字节）。

#### Scenario: 零拷贝反序列化 String 列
- **WHEN** 输入 data 含 `0x02 [len u16 LE] [N bytes UTF-8]` 且 schema 为 `[String(N)]`
- **THEN** 返回 `Vec<ValueRef<'a>>` 含 1 个元素 `ValueRef::Text(&'a str)`，str 借用 data 内部
- **AND** 函数内部对 String 列 MUST NOT 调用 `String::from_utf8(... .to_vec())`

#### Scenario: 零拷贝反序列化 Int/Float/Bool/Null 列
- **WHEN** 输入 data 含 TAG_INT / TAG_FLOAT / TAG_BOOL / TAG_NULL
- **THEN** 返回对应 `ValueRef::Int(i64)` / `ValueRef::Float(f64)` / `ValueRef::Bool(bool)` / `ValueRef::Null`
- **AND** 全部零分配（stack-only）

#### Scenario: 错误输入返回 StorageError
- **WHEN** data 截断（期望 tag 字节缺失 / 期望 N 字节不足）
- **THEN** 返回 `Err(StorageError::Io(UnexpectedEof))`
- **WHEN** data 含未知 tag byte
- **THEN** 返回 `Err(StorageError::Io(InvalidData))`
- **WHEN** TAG_STRING 字节不是合法 UTF-8
- **THEN** 返回 `Err(StorageError::Io(InvalidData))`

### Requirement: ValueRef<'a> MUST 支持 to_value() 转换

`ValueRef<'a>` MUST 实现 `to_value(&self) -> Value` 方法，转换时：
- `Int / Float / Bool / Null` 零分配
- `Text(&str)` MUST 调用 `to_string()` 分配一次 `String`

#### Scenario: Text 借用 to_value
- **WHEN** `ValueRef::Text("hello")` 调用 `to_value()`
- **THEN** 返回 `Value::String(String)` 含 `"hello"`，分配 1 次 String

#### Scenario: Int/Float/Bool/Null to_value 零分配
- **WHEN** `ValueRef::Int(42)` / `ValueRef::Float(1.5)` / `ValueRef::Bool(true)` / `ValueRef::Null` 调用 `to_value()`
- **THEN** 返回对应 `Value` 变体，零堆分配

### Requirement: Expression trait MUST 提供 evaluate_ref 零拷贝路径

`Expression` trait MUST 新增 `fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, _>` 方法。`fn evaluate(&self, row: &[Value])` MUST 由 trait 默认方法实现，内部转调 `evaluate_ref().to_value()`，**不**依赖具体实现的覆盖。

#### Scenario: 3 个 Expression 实现都补 evaluate_ref
- **WHEN** `ColumnExpression` / `ConstantExpression` / `ParameterExpression` 编译
- **THEN** 每个实现 MUST 包含 `fn evaluate_ref(...)` 否则编译失败

#### Scenario: evaluate 默认实现调用 evaluate_ref + to_value
- **WHEN** 调用 `expr.evaluate(row: &[Value])`
- **THEN** 默认实现将 `row` 转为 `Vec<ValueRef>` via `Value::as_value_ref`，调 `evaluate_ref(&row_ref).to_value()`
- **AND** 与原实现语义等价

#### Scenario: 3 个 Expression evaluate_ref 不允许 .await / 递归 BufferPool
- **WHEN** 任意 Expression 的 `evaluate_ref` 实现
- **THEN** MUST NOT call `.await`
- **AND** MUST NOT recursively call BufferPool methods（防止 deadlock）
- **THEN** borrow checker 强制（`FnOnce` 而非 `async`）

### Requirement: Scan 执行器 MUST 使用零拷贝 deserialize_value_refs 路径

`ScanExecutor` / `IndexScanExecutor` / `IndexScanAllExecutor` 的 tuple 读取闭包 MUST 改用 `deserialize_value_refs` + `to_value()` 模式，替代 `deserialize_tuple`。

#### Scenario: 3 个 Scan 闭包改造完成
- **WHEN** 编译
- **THEN** 3 个执行器的 `find_visible_version` / `read_tuple_from_data_page` 闭包内 MUST 使用 `deserialize_value_refs` 而非 `deserialize_tuple`

#### Scenario: 闭包内 String 列零分配
- **WHEN** Scan 处理 1K 行 × 1 String 列 × 平均 300B
- **THEN** 闭包内对 String 列 MUST NOT 分配 30万次（`AllocationCounter` 验证）
- **AND** 闭包内 MUST 仅在 `to_value()` 调用点分配 1K 次 String

### Requirement: Value::as_value_ref MUST 提供 owned-to-borrowed 视图

`Value::as_value_ref(&self) -> ValueRef<'_>` MUST 提供 owned Value 到 borrowed `ValueRef` 的转换。`Value::String(s)` 转 `ValueRef::Text(s.as_str())` 借用 `s` 内部，不分配。

#### Scenario: 所有 Value 变体的 as_value_ref
- **WHEN** `Value::Int(42)` / `Value::String("hi")` / `Value::Null` / `Value::Float(1.5)` / `Value::Bool(true)` 调用 `as_value_ref()`
- **THEN** 返回对应 `ValueRef::Int(42)` / `ValueRef::Text("hi")` / `ValueRef::Null` / `ValueRef::Float(1.5)` / `ValueRef::Bool(true)`
- **AND** 全部零分配（String 变体借用 s 内部）

### Requirement: M36 MUST NOT 修改 Value 枚举或 deserialize_tuple

为避免与 M37 (clone 消除 Arc<str>) 范围重叠，M36 严格不动 `Value` 枚举内部。`deserialize_tuple` MUST 保持不变（向后兼容 + 写路径用）。

#### Scenario: Value::String 保持 String 内部
- **WHEN** M36 实施
- **THEN** `Value::String` 内部 MUST 仍是 `String`，不变 `Arc<str>`

#### Scenario: deserialize_tuple 签名保持
- **WHEN** M36 实施
- **THEN** `deserialize_tuple(data: &[u8], schema: &[ColumnType]) -> Result<Vec<Value>>` 签名 MUST 不变
- **AND** 现有调用方（M20 留下的 5 处 executor_test 调用）MUST 仍编译通过
