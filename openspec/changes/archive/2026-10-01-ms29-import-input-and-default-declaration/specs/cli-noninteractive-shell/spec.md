# cli-noninteractive-shell Delta — import 输入面（stdin / 免建表导入 / `:memory:` 豁免）

## MODIFIED Requirements

### Requirement: 生命周期子命令 import --csv

`rtsql import <db> <table> <file> --csv` SHALL 将 CSV 数据导入数据库：`file` SHALL 接受文件路径或 `-`（stdin，对齐 `restore` 通道先例）。CSV 首行为表头。目标表**已存在**时沿既有语义：按列名与目标表列匹配（顺序无关），目标表全部列 SHALL 在表头中出现（缺失或未知列 SHALL 以退出码 1 拒绝）；字段值 SHALL 按目标表列类型转换：Int 列按 i64 解析、Float 列按 f64 解析、Bool 列接受 true/false（大小写不敏感）、String 列原样使用；空字段 SHALL 转换为 NULL（非 String 列）或空字符串（String 列）。转换失败 SHALL 以退出码 1 fail-fast 报错并含数据行定位；SQL 执行失败 SHALL 以退出码 3 fail-fast 报错；两者失败前已提交的行保留（逐条 auto-commit）。成功时 SHALL 输出导入行数（沿用受影响行数渲染语义）并以退出码 0 退出。锁冲突 SHALL 以退出码 4 报错。CSV 解析 SHALL 遵循 RFC4180（引号、转义、跨行字段）。

目标表**不存在**时 SHALL 按表头自动建表：列名与列序 SHALL 取 CSV 表头；列类型 SHALL 按全部数据行逐字段猜测（可解析为 i64 → INT、可解析为 f64 → FLOAT、其余字段一律按 STRING 参与推导）并按 CTAS 推导矩阵规则合并（同型保留、INT∪FLOAT → FLOAT、NULL 参与任意类型、全 NULL 或零数据行的列落 STRING）；列内类型冲突 SHALL 以点名列名与冲突类型的错误拒绝（不做 STRING 宽化）。自动建表 SHALL NOT 显式声明主键——新表 SHALL 沿用引擎「未声明主键 → 首列隐式主键」既有语义（与 `create-table-as-select` 物化表同款，`rtsql schema` 渲染首列 `PRIMARY KEY`）：首列声明类型为 INT 时行值重复 SHALL 以 DuplicateKey fail-fast 拒绝（失败前已提交行保留，逐条 auto-commit 既有语义），首列为 FLOAT/STRING 时行落非键行语义、重复值合法。推导 SHALL 在收齐数据行后、建表前完成：任何推导失败 SHALL 以退出码 1 拒绝且零副作用（目标表不存在于失败结果中）；自动建表 SHALL 经常规建表通道执行（保留名/重名视图/重复列名等计划期守卫自然生效），建表失败 SHALL 以既有 SQL 失败通道报错且零副作用。自动建表成功后 SHALL 沿既有导入路径逐行写入（空字段语义沿用目标表导入规则：推断为 STRING 的列空字段插入空字符串，其余列空字段插入 NULL）。

`:memory:` 组合 SHALL 豁免生命周期子命令的内存库拒绝：`rtsql import :memory: <table> <file|-> --csv` SHALL 可达（内存库打开，进程退出即弃，`in-memory-database` 语义零特殊化）；`--key` 组合 SHALL 以退出码 2 拒绝（内存库永不加密，对齐主命令语义）。文件库目标缺失时 SHALL 维持既有退出码 1 拒绝（import 不创建数据库文件，创建通道收敛 `rtsql new`）。

#### Scenario: 基本导入

- **GIVEN** 表 `users (id INT PRIMARY KEY, name STRING)`；CSV 文件首行 `id,name`、数据两行
- **WHEN** `rtsql import app users data.csv --csv`（非 TTY）
- **THEN** 表内新增 2 行且值正确，stdout 输出 `{"affected_rows":2}`，退出码 0

#### Scenario: 表头乱序匹配

- **GIVEN** 同上表；CSV 首行 `name,id`（顺序与表定义相反）
- **WHEN** `rtsql import app users data.csv --csv`
- **THEN** 各值按列名正确映射入库，退出码 0

#### Scenario: 类型转换与空字段语义

- **GIVEN** 表 `t (a INT, b FLOAT, c BOOL, d STRING)`；CSV 数据行含空字段与合法类型文本
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** `a` 空字段为 NULL、`b`/`c` 按类型解析、`d` 空字段为空字符串；SELECT 比对验证，退出码 0

#### Scenario: 转换失败 fail-fast

- **GIVEN** 表 `t (a INT)`；CSV 数据第 2 行 `a` 列为 `abc`
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** 退出码 1，stderr 含失败数据行定位；第 1 行已入库（重开可见）

#### Scenario: 表头列不匹配拒绝

- **GIVEN** 表 `t (a INT, b INT)`；CSV 表头缺 `b` 列或含表外列
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** stderr 报错，退出码 1，表内无新行

#### Scenario: 目标表或库不存在拒绝

- **GIVEN** 库 `missing` 不存在
- **WHEN** `rtsql import missing t data.csv --csv`（或 stdin 形态 `rtsql import missing t - --csv`）
- **THEN** stderr 报错库 `does not exist`，退出码 1，零文件创建（创建通道仍收敛 `rtsql new`）
- **AND** 库存在而目标表缺失的形态不再拒绝：按上方自动建表场景语义建表导入（原「表不存在退出码 1」拒绝面由自动建表语义承接，见「目标表不存在自动建表（类型推导）」等场景）

#### Scenario: CSV 引号转义字段

- **GIVEN** CSV 字段含逗号、双引号与跨行文本（RFC4180 引号转义）
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** 字段值完整入库（SELECT 比对），退出码 0

#### Scenario: import 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql import <db> <table> data.csv --csv`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4

#### Scenario: stdin 导入既有表

- **GIVEN** 表 `t (a INT, b STRING)`；进程管道 `printf 'a,b\n1,x\n2,y\n' | rtsql import app t - --csv`
- **WHEN** import 从 stdin 读取 CSV
- **THEN** 导入 2 行，stdout 输出 `{"affected_rows":2}`，退出码 0；SELECT 比对值正确

#### Scenario: 目标表不存在自动建表（类型推导）

- **GIVEN** 库 `app` 无表 `t`；CSV 首行 `id,score,label`、数据行含整数、浮点与文本
- **WHEN** `rtsql import app t data.csv --csv`（或 stdin 形态）
- **THEN** 表 `t` 按 `(id INT, score FLOAT, label STRING)` 创建（INT∪FLOAT → FLOAT 提升矩阵生效），全部行导入；`rtsql schema app` 可见该表；SELECT 比对值与类型正确

#### Scenario: 同型列推导与全 NULL 列

- **GIVEN** CSV 表头 `a,b`：`a` 列数据全为整数文本、`b` 列数据全为空字段
- **WHEN** 对无表目标自动建表导入
- **THEN** `a` 推导为 INT、`b`（全 NULL）落 STRING；导入成功退出码 0

#### Scenario: 零数据行（仅表头）自动建表

- **GIVEN** CSV 仅含表头行 `a,b,c`
- **WHEN** 对无表目标自动建表导入
- **THEN** 表按三列全 STRING 创建，导入 0 行，退出码 0

#### Scenario: 类型冲突点名拒绝零副作用

- **GIVEN** CSV 表头 `a`：数据行既有 `1`、`2` 又有 `abc`
- **WHEN** 对无表目标自动建表导入
- **THEN** 退出码 1，stderr 点名列名 `a` 与冲突类型（INT 与 STRING）；目标表不存在（`rtsql schema` 零输出），零副作用

#### Scenario: 自动建表首列隐式主键（引擎语义沿用）

- **GIVEN** CSV 表头 `id,v`：`id` 列数据含重复整数值（如 `1` 出现两行）；对照 CSV 首列推导为 FLOAT/STRING 且含重复值
- **WHEN** 对无表目标自动建表导入
- **THEN** INT 首列形态于首个重复值处以 DuplicateKey fail-fast 拒绝（exit 3，失败前已提交行保留——引擎「未声明主键 → 首列隐式主键」语义，与 `create-table-as-select` 同款；`rtsql schema` 渲染首列 `PRIMARY KEY`）；FLOAT/STRING 首列重复值全部导入成功（非键行语义，不触发 DuplicateKey）

#### Scenario: 自动建表经保留名/重复列名守卫拒绝

- **GIVEN** CSV 表头含重复列名（如 `a,a`）
- **WHEN** 对无表目标自动建表导入
- **THEN** 经常规建表通道计划期守卫拒绝（SQL 失败通道，退出码 3），零副作用

#### Scenario: `:memory:` import 免建表可达

- **WHEN** `printf 'a,b\n1,x\n' | rtsql import :memory: t - --csv`
- **THEN** 内存库自动建表并导入，stdout 输出 `{"affected_rows":1}`，退出码 0（进程退出即弃，无任何文件产生）

#### Scenario: `:memory:` import `--key` 拒绝

- **WHEN** `rtsql import :memory: t - --csv --key secret`
- **THEN** 退出码 2，stderr 点名内存库不支持加密（对齐主命令语义）

#### Scenario: `:memory:` 其他生命周期子命令维持拒绝

- **WHEN** `rtsql :memory: schema`、`rtsql :memory: dump` 等（import 以外的生命周期子命令）
- **THEN** 维持既有退出码 2 拒绝（豁免面仅 import）
