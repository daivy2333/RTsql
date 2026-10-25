# 旧格式兼容样本（legacy fixtures）

本目录是 **MS36「历史库与 dump 兼容」**（change
`2026-10-24-ms36-legacy-format-and-dump-compat`，Iteration 000 / Cycle 000，T1）
的固定输入样本。

样本全部由**更早版本的产品构建**产出的真实字节，作为固定输入提交进仓库；
`tests/legacy_compat_test.rs` 在运行时把它们复制到临时目录再打开/恢复。
测试**不依赖旧工具链**，也不在运行期做任何版本 / 哈希 / 修订比对——
样本只是固定输入。

## 样本来源

| 项 | 值 |
|---|---|
| 旧构建 revision | `d8165fc634fa650d839811b1d6b0165e204cad01`（2026-09-24，MS23 落库前的最后一次提交） |
| 旧构建方式 | `git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy && git checkout d8165fc && cargo build --offline --bin rtsql`（构建在仓库之外的隔离目录，不触碰工作区） |
| 旧构建产物 | `/tmp/rtsql-legacy/target/debug/rtsql` |
| 工具链 | rustc 1.99.1 / cargo 1.99.1（`cargo build --offline`） |
| 旧构建的格式代际 | G1（最旧目录行代际）：`__tables` 行止于数据链尾指针（无唯一根段 / 无二级索引段 / 无表类型字节），`__columns` 行无 DEFAULT 段 |

格式版本号自产品引入文件头起一直是 `1`——**不存在更旧的格式版本号**；
「旧格式」不是版本号维度，而是目录行布局代际。旧行是当前行的字节前缀
（新段一律追加在行尾），因此旧库可被当前构建的兼容读分支解析。

## 样本内容清单

| 文件 | 字节 | 内容 | 覆盖的 Acceptance |
|---|---|---|---|
| `legacy_g1.db` | 41024 | 旧构建建的 G1 库：活表 `users`（3 行）、`audit`（1 行）、`events`（1 行，含 DATE / TIMESTAMP / BOOL）；另有 `discarded` 表由**旧构建**删掉（遗留空闲页） | A1 / A2 / A3 / A4 |
| `legacy_g1_pending_wal.db` | 28736 | 旧构建建的 G1 库：表 `t`（1 行种子数据） | A5 |
| `legacy_g1_pending_wal.wal` | 15093 | 与上库同名的**未决写入日志**（旧构建 import 中途被 `SIGKILL`，未 checkpoint） | A5 |
| `legacy_dump.sql` | 497 | 旧构建的 `dump` 输出：`CREATE TABLE` + `INSERT`，短类型名 `INT` / `STRING` / `BOOL`，含 `DATE '…'` / `TIMESTAMP '…'` / `TRUE` 字面量 | A6 |
| `legacy_dump_non_integer_pk.sql` | 98 | 旧构建的 `dump` 输出：非整数（TEXT）主键表 | A7 |
| `legacy_dump_non_integer_unique.sql` | 107 | 旧构建的 `dump` 输出：非整数（TEXT）列上的 `UNIQUE` | A7 |
| `legacy_dump_null_in_not_null.sql` | 133 | 旧 dump 文本的一处**最小编辑**：把第 2 条 INSERT 的值改成 `NULL`，落到 `NOT NULL` 列上 | A7 |
| `legacy_g2.db` | 32832 | 旧构建（revision `6e8dcf5`）建的 **G2** 库：活表 `people`（3 行）、`notes`（1 行，`tag` 列声明 `DEFAULT 'none'`）；另有 `scratch` 表由**旧构建**删掉（遗留空闲页） | C-1 / C-2 |

### 各样本的预期结论（由 `tests/legacy_compat_test.rs` 断言）

- `legacy_g1.db`：当前构建可打开、查询、写入；写入后被触及的表目录行升级为
  当前布局（唯一根计数 0 / 二级索引计数 0 / 表类型字节 0x01），未被写到的表
  目录行保持旧代际；打开时重建不放弃（stderr 无「重建放弃」），活表完整，
  完整性扫描 error 级发现数为 0。
- `legacy_g1_pending_wal.db` + `.wal`：当前构建打开后从未决日志恢复出已提交行，
  完整性扫描 error 级发现数为 0。**恢复行数参考值：133 行**（表 `t` 的 1 行种子
  数据 + 132 行导入数据）。该数字是**已提交样本字节的属性**（确定）：同一份
  样本字节恢复出的行集恒定。用例断言「恢复行数 == 该参考值」——只断言
  「行数 ≥ 1」时，未决日志未被重放也会通过（样本自带 1 行种子数据），断言
  就没有判别力。若重新生成样本（旧构建被杀时机不同 → 参考值改变），请同步
  更新本行与用例中的参考值常量。
- `legacy_dump.sql`：恢复到当前空库成功，行数据等价；恢复出的库可再次导出并
  保持等价。
- 三份范围外 dump：恢复失败，退出码 **3**，错误文本点名原因并定位到语句
  （`statement k of n failed: … ; statement: …`）：
  - 非整数主键 → `Plan error: Unsupported constraint: NON-INTEGER PRIMARY KEY`（第 1 条 / 共 2 条）
  - 非整数 `UNIQUE` → `Plan error: Unsupported constraint: UNIQUE (INT columns only)`（第 1 条 / 共 2 条）
  - `NOT NULL` 含 `NULL` → `Execution error: NOT NULL constraint violation: column 's'`（第 3 条 / 共 3 条）

### G2 代际样本（`legacy_g2.db`）

G2 是 G1 之后的**目录行布局代际**：`__tables` 行在 G1 固定段之后追加
`u32 唯一根计数` 与 `唯一根数 × u32 唯一根`；该段之后**没有**二级索引段与
表类型字节（当前布局才有）。`__columns` 行可带 MS24 的 DEFAULT 段。

| 项 | 值 |
|---|---|
| 旧构建 revision | `6e8dcf551aa53d2a08dc6f20097f1ad5e50fc464`（2026-09-26，MS23+MS24 收尾统一入库） |
| 旧构建方式 | `git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy-g2 && git checkout 6e8dcf5 && cargo build --offline --bin rtsql`（构建在**仓库之外**的隔离目录，不触碰工作区） |
| 旧构建产物 | `/tmp/rtsql-legacy-g2/target/debug/rtsql` |
| 工具链 | rustc 1.99.1 / cargo 1.99.1（`cargo build --offline`） |
| 表清单 | `people`（3 行）、`notes`（1 行）；`scratch` 由旧构建删掉，不在目录里 |
| `__columns` 行 | 6 行（2 表 × 3 列）；只有 `notes.tag` 带 DEFAULT 段 |

由 `tests/legacy_compat_test.rs` 断言的派生量（与本节写同一组数字）：

- `__tables` 行长度 = G1 固定段 + **4 字节**（唯一根计数 = 0）；该 4 字节之后
  **没有**别的段——即目录行不含二级索引段与表类型字节。
- `notes.tag` 的 `__columns` 行长度 = G1 固定段 + **8 字节** DEFAULT 段
  （`u8 有默认值` + `u8 字符串标签` + `u16 长度` + `"none"` 四字节），段内解出
  字面量 `'none'`；其余 5 个列行止于 G1 固定段（无 DEFAULT 段）。
- 当前构建可打开 / 查询 / 写入该库；写入把**被触及**的表目录行升级为当前
  布局，未触及的表目录行仍停在 G2 代际；CLI 打开时重建不放弃（stderr 无
  「重建放弃」）；完整性扫描 error 级发现数为 0。
- 当前构建可 `dump` 该库，导出的文本可恢复到新库且行集等价（含声明 DEFAULT
  的列）。

## 再生步骤

全部在**仓库之外**的隔离目录执行（不触碰工作区现场）：

```bash
# 1) 隔离克隆并构建旧版本
git clone --no-hardlinks --shared /home/daivy/projects/RTsql /tmp/rtsql-legacy
cd /tmp/rtsql-legacy && git checkout d8165fc
cargo build --offline --bin rtsql
OLD=/tmp/rtsql-legacy/target/debug/rtsql

# 2) G1 库文件样本（活表 users / audit / events + 由旧构建删掉的 discarded）
F=/tmp/legacy-fixture
rm -rf $F && mkdir -p $F
$OLD new $F/legacy.db
$OLD $F/legacy.db "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score FLOAT);"
$OLD $F/legacy.db "INSERT INTO users VALUES (1, 'alice', 9.5), (2, 'bob', 7.25), (3, 'carol', 8.0);"
$OLD $F/legacy.db "CREATE TABLE audit (id INTEGER PRIMARY KEY, note TEXT);"
$OLD $F/legacy.db "INSERT INTO audit VALUES (1, 'kept');"
$OLD $F/legacy.db "CREATE TABLE events (id INTEGER PRIMARY KEY, day DATE, at TIMESTAMP, ok BOOL);"
$OLD $F/legacy.db "INSERT INTO events VALUES (1, DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05', TRUE);"
$OLD $F/legacy.db "CREATE TABLE discarded (id INTEGER PRIMARY KEY, note TEXT);"
$OLD $F/legacy.db "INSERT INTO discarded VALUES (1, 'legacy-dropped');"
$OLD $F/legacy.db "DROP TABLE discarded;"

# 3) 旧 dump 文本样本（主样本）
$OLD dump $F/legacy.db > $F/legacy_dump.sql

# 4) G1 库 + 未决写入日志样本（import 中途 SIGKILL，不 checkpoint）
$OLD new $F/pending.db
$OLD $F/pending.db "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);"
$OLD $F/pending.db "INSERT INTO t VALUES (0, 'seed');"
{ echo "id,v"; for i in $(seq 1 3000); do echo "$i,row-$i"; done; } > $F/rows.csv
$OLD import $F/pending.db t $F/rows.csv --csv & PID=$!
sleep 0.35; kill -9 $PID; wait $PID 2>/dev/null || true

# 5) 三类范围外旧 dump 文本样本
$OLD new $F/nipk.db
$OLD $F/nipk.db "CREATE TABLE nipk (id TEXT PRIMARY KEY, v TEXT);"
$OLD $F/nipk.db "INSERT INTO nipk VALUES ('k1', 'x');"
$OLD dump $F/nipk.db > $F/legacy_dump_non_integer_pk.sql

$OLD new $F/niuq.db
$OLD $F/niuq.db "CREATE TABLE niuq (id INTEGER PRIMARY KEY, email TEXT UNIQUE);"
$OLD $F/niuq.db "INSERT INTO niuq VALUES (1, 'a@b.c');"
$OLD dump $F/niuq.db > $F/legacy_dump_non_integer_unique.sql

$OLD new $F/nn.db
$OLD $F/nn.db "CREATE TABLE nn (id INTEGER PRIMARY KEY, s TEXT NOT NULL);"
$OLD $F/nn.db "INSERT INTO nn VALUES (1, 'ok'), (2, 'also ok');"
$OLD dump $F/nn.db > $F/nn_base.sql
sed "s/INSERT INTO \"nn\" VALUES (2, 'also ok');/INSERT INTO \"nn\" VALUES (2, NULL);/" \
    $F/nn_base.sql > $F/legacy_dump_null_in_not_null.sql

# 6) 落到仓库（只取样本本体；伴生 .wal / .checkpoint 不入库）
cp $F/legacy.db    tests/fixtures/legacy/legacy_g1.db
cp $F/pending.db   tests/fixtures/legacy/legacy_g1_pending_wal.db
cp $F/pending.wal  tests/fixtures/legacy/legacy_g1_pending_wal.wal
cp $F/legacy_dump.sql                    tests/fixtures/legacy/
cp $F/legacy_dump_non_integer_pk.sql     tests/fixtures/legacy/
cp $F/legacy_dump_non_integer_unique.sql tests/fixtures/legacy/
cp $F/legacy_dump_null_in_not_null.sql   tests/fixtures/legacy/
```

### G2 样本的再生步骤

同样全部在**仓库之外**执行（旧构建来自 G2 代际对应的 revision）：

```bash
# 1) 隔离克隆并构建 G2 代际的旧版本
git clone --no-hardlinks --shared /home/daivy/projects/RTsql /tmp/rtsql-legacy-g2
cd /tmp/rtsql-legacy-g2 && git checkout 6e8dcf5
cargo build --offline --bin rtsql
OLD=/tmp/rtsql-legacy-g2/target/debug/rtsql

# 2) G2 库文件样本（活表 people / notes + 由旧构建删掉的 scratch）
F=/tmp/legacy-fixture-g2
rm -rf $F && mkdir -p $F
$OLD new $F/legacy_g2.db
$OLD $F/legacy_g2.db "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score FLOAT);"
$OLD $F/legacy_g2.db "INSERT INTO people VALUES (1, 'ada', 9.5), (2, 'bob', 7.25), (3, 'cyd', 8.0);"
$OLD $F/legacy_g2.db "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, tag TEXT DEFAULT 'none');"
$OLD $F/legacy_g2.db "INSERT INTO notes (id, body) VALUES (1, 'first note');"
$OLD $F/legacy_g2.db "CREATE TABLE scratch (id INTEGER PRIMARY KEY, tmp TEXT);"
$OLD $F/legacy_g2.db "INSERT INTO scratch VALUES (1, 'to be dropped');"
$OLD $F/legacy_g2.db "DROP TABLE scratch;"

# 3) 落到仓库（只取样本本体）
cp $F/legacy_g2.db tests/fixtures/legacy/legacy_g2.db
```

## 已知限制

- **未决日志样本的恢复行数**：已提交样本的恢复行数是**确定值**（133 行，见上），
  用例断言「恢复行数 == 参考值 + 完整性零 error」；该断言在**未决日志未被重放**
  时失败（此时只有 1 行种子数据）。只有**重新生成**样本（旧构建被杀时机不同）
  才会改变这个数字——那时需同步更新上表参考值与用例中的参考值常量。
- **`legacy_dump_null_in_not_null.sql` 是旧 dump 文本的一处最小编辑**：旧构建
  自身拒绝把 `NULL` 写进 `NOT NULL` 列，因此该样本由旧构建的 dump 文本派生
  （只改一条 INSERT 的值），其余字节保持旧构建产出形态。
- **`discarded` 表的空闲页**：旧构建删表后不写空闲页清单；当前构建在打开时按
  持久目录重建可分配页集合（MS35），这正是 `legacy_g1.db` 覆盖「打开时重建
  不放弃」的前提。
- **样本规模**：行数 ≪ 300（`users` 3 行、`audit` 1 行、`events` 1 行、
  `t` 1 行种子 + 132 行恢复），避开 ISS23 记录的栈溢出触发面
  （`ALTER TABLE DROP COLUMN` 的完成路径，与本场景无关）。
- **不覆盖**：加密旧库的专项断言（当前构建打开旧构建的加密库已顺带验证可行，
  但不作为独立断言项）；`dump` 对非整数 `UNIQUE` 列无条件渲染 `UNIQUE` 导致的
  「旧库 → dump → 新库 restore」被拒（既有行为，见 change design D9）。
