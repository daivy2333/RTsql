## Purpose

规定可避免整组复制的测量，以及恰好一处达到执行器耗时 10% 时的实现。未达到时不改产品代码。

## ADDED Requirements

### Requirement: 整组克隆和字符串克隆的耗时可以打印

系统 SHALL 能对下列四个执行器负载读出克隆循环耗时和执行器排空耗时。负载在测试进程内构造，不经过 SQL 解析。计时使用一次 `Instant` 包裹整段循环或整段排空，不在每一行克隆上各取一次时间。克隆循环的结果 SHALL 经 `std::hint::black_box` 使用，避免循环被删掉。

1. 哈希连接。左右各 50 行。左行是 `(Int(i), String("customer_{i}"))`，右行是 `(Int(i), Int(i * 100))`，`i` 从 0 到 49。等值条件是两侧第 0 列。输出两侧全部列。探测 SHALL 按当前实现按下标取匹配桶里的行，不克隆整个匹配桶。这个负载的桶克隆次数为 0，`hash_bucket_clone_ns` 为 0。
2. 嵌套循环内连接。左右各 50 行，行形状与哈希连接的左行相同。谓词是恒真。输出两侧全部列。组合行克隆循环执行 2500 次，每次克隆当时的左行和右行。
3. 排序。100 行，形状为 `(Int(i), String("user_{i}"))`，按 `i` 从 99 到 0 送入。排序键是第 0 列升序，`expr` 为空。交出克隆循环把这 100 行各克隆一次。
4. 排序合并。左右各 50 行，行形状与哈希连接相同，等值条件是两侧第 0 列。交出克隆循环把 50 个输出行各克隆一次。输出行的列就是执行器输出的列。

同一测试还 SHALL 把 `customer_0` 到 `customer_49` 各克隆一次，打印 `string_clone_ns`。

测试 SHALL 打印 `hash_executor_ns`、`hash_bucket_clone_ns`、`nlj_executor_ns`、`nlj_clone_ns`、`sort_executor_ns`、`sort_clone_ns`、`smj_executor_ns`、`smj_clone_ns`、`string_clone_ns`，以及 `hash_rows`、`nlj_rows`、`sort_rows`、`smj_rows`。打印本身 SHALL NOT 判定是否达到 10%。

#### Scenario: 四个执行器给出耗时且行数正确

- **GIVEN** 上述四个负载
- **WHEN** 各排空一次
- **THEN** `hash_rows` 为 50，`nlj_rows` 为 2500，`sort_rows` 为 100，`smj_rows` 为 50
- **AND** 排序输出的首行第 0 列是 `Int(0)`，末行第 0 列是 `Int(99)`
- **AND** 输出中有本 Requirement 列出的全部纳秒字段
- **AND** 四个执行器耗时都大于 0

#### Scenario: 执行器耗时为零或乘法溢出时测量无效

- **GIVEN** 上述负载
- **WHEN** 任一执行器排空耗时为 0，或 `clone_ns * 10` 在 `u128` 上溢出
- **THEN** 这次测量不能用来裁定
- **AND** 不得把该次结果记成低于 10%

### Requirement: 达到百分之十才改一处复制

10% 是本变更是否改产品代码的条件，不是运行时分支。库 SHALL NOT 在执行查询时读取这些纳秒数来选择路径。

比较使用 `u128`。某一处达到门槛，当且仅当它的 `clone_ns * 10 >= executor_ns`。字符串克隆的分母是 `hash_executor_ns`，分子是 `string_clone_ns`。

裁定顺序 SHALL 为：

1. 在哈希桶、嵌套循环组合行、排序交出、排序合并交出这四处里，找出所有达到门槛的。
2. 有达到的，只改 `clone_ns` 最大的一处。`clone_ns` 相等时依次取嵌套循环、排序合并、排序、哈希桶。`Value::String` 保持 `String`。
3. 四处都未达到，且字符串克隆达到门槛时，只把 `Value::String` 的负载从 `String` 改为 `Arc<str>`。
4. 都未达到时，不改产品代码。

只允许第 2 步或第 3 步其中之一发生。已经改了一处之后，SHALL NOT 再改另一处。

#### Scenario: 嵌套循环达到且克隆耗时最大

- **GIVEN** `nlj_clone_ns * 10 >= nlj_executor_ns`，并且 `nlj_clone_ns` 不小于其他已达到门槛的整组克隆耗时
- **WHEN** 按本 Requirement 裁定
- **THEN** 只改嵌套循环的组合行构造
- **AND** `Value::String` 的负载仍是 `String`

#### Scenario: 整组克隆都未达到而字符串克隆达到

- **GIVEN** 四处整组克隆都是 `clone_ns * 10 < executor_ns`，且 `string_clone_ns * 10 >= hash_executor_ns`
- **WHEN** 按本 Requirement 裁定
- **THEN** 只改 `Value::String` 的负载
- **AND** 四个执行器的整组克隆方式保持测量前的实现

#### Scenario: 全部低于百分之十

- **GIVEN** 四处整组克隆和字符串克隆都低于 10%
- **WHEN** 结束本次实现
- **THEN** 产品代码与测量前一致
- **AND** 变更记录里有打印出的纳秒数和「保留原实现」

### Requirement: 改动后的 SQL 结果与测量前一致

实现发生时，同一输入上的行多重集合、排序次序、值的相等和顺序、错误文本与 SQLSTATE SHALL 与改前一致。磁盘元组的标签和字节布局 SHALL 不变。诊断阶段名 SHALL 不变。

嵌套循环若被选中，组合行 SHALL 仍按左行各列后接右行各列交给现有谓词。构造这个视图时 SHALL 移动值，求值结束后把值移回原来的左右行。谓词返回后 SHALL NOT 仍借用该行。

排序或排序合并若被选中，交出一行时 SHALL 把已经存下的那一行移出，而不是再克隆一行。移出后的空位 SHALL NOT 再被读取。输出次序 SHALL 与移出前一致。

`Value::String` 若被选中，相等、哈希、顺序和显示 SHALL 仍按字符串内容。其他 `Value` 变体 SHALL 不变。`ValueRef::Text` 的借用语义 SHALL 不变。

#### Scenario: 嵌套循环的非等值连接结果不变

- **GIVEN** 裁定选择了嵌套循环，两张表各有整数列和非等值连接条件
- **WHEN** 执行该连接
- **THEN** 结果行多重集合与改前一致
- **AND** 错误文本与 SQLSTATE 与改前一致

#### Scenario: 排序次序不变

- **GIVEN** 裁定选择了排序或排序合并
- **WHEN** 对同一输入执行排序或排序合并
- **THEN** 输出次序和单元格与改前一致

#### Scenario: 字符串值的比较不变

- **GIVEN** 裁定选择了 `Value::String`
- **WHEN** 比较、连接和分组使用相同的字符串内容
- **THEN** 相等、顺序和分组结果与负载仍是 `String` 时一致
- **AND** 写入后再读回的元组字节与改前一致

### Requirement: 分配器、行锁和诊断保持原实现

本变更 SHALL NOT 引入 `jemalloc` 或 `mimalloc`，SHALL NOT 修改 `RowLockTable`，SHALL NOT 新增诊断阶段或诊断计数。文件锁的 `DatabaseLocked` 与锁等待预算 SHALL 不变。可见性判断 SHALL 不变。

#### Scenario: 测量结束时分配器与行锁表仍是原实现

- **GIVEN** 测量测试已经打印纳秒数
- **WHEN** 按裁定结束本次实现，无论是否改了复制
- **THEN** 构建依赖里没有新的全局分配器
- **AND** `RowLockTable` 仍用 `RwLock<HashMap<...>>`
- **AND** 诊断摘要的阶段名与改前一致
