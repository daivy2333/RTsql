# RTsql vs SQLite 跨引擎对比基准与资源测量

- Status: active
- Last validated: 2026-09-24
- Environment: Linux x86_64（Intel i9-13900HX，32 线程）、Rust 2021 / Cargo（release + bench profile，`profile.release` lto=true）、criterion 0.5、rusqlite 0.31（链接系统 libsqlite3 3.37.2）、sqlite3 CLI 3.37.2、GNU time（/usr/bin/time -v）、python3、coreutils stat/date
- Source: 外部执行证据（2026-09-24 本会话命令与输出；工作树基于 commit `1a2d877`，README 对比板块写入时未提交）；结果已固化为 `README.md` / `README.zh-CN.md`「性能与资源对比」板块

## 适用范围

- 需要 RTsql 对 SQLite 的性能与资源占用快照时：引擎级吞吐/延迟（INSERT、主键点查、全表扫描）+ CLI 级资源面（加载耗时、峰值 RSS、库文件体积、one-shot 启动时延、二进制体积）。
- 适用于 README/文档对比板块刷新、发布前快照采集。
- 不适用：RTsql 自身改动前后的回归判读（用 R17 `ms08-bench-comparison.md` 的 baseline 流程）；正确性验证（用 `cargo test`）；可下结论的正式性能结论（本流程只产出单机观察）。

## 前置条件

- `cargo build --release --bin rtsql` 完成（bench profile 与 release 同参编译，target 暖时可忽略）。
- `sqlite3` CLI 与系统 `libsqlite3` 开发头可用（rusqlite 默认非 bundled，链接系统库；版本一致才有代表性）。
- `/usr/bin/time -v`（GNU time）、python3、GNU stat/date 可用。
- 公平口径：两引擎均默认配置（SQLite 默认回滚日志 + 每语句隐式事务；RTsql 每语句经 WAL 自动提交），同一合成表 `bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)`；声明该口径，不做配置调优对齐。

## 操作步骤

1. **引擎级 criterion（三段分跑，降低单次等待）**
   ```bash
   cargo bench --bench sqlite_compare -- compare_insert --measurement-time 2 --warm-up-time 0.5 --sample-size 20 --noplot
   cargo bench --bench sqlite_compare -- compare_pk_lookup --measurement-time 2 --warm-up-time 0.5 --sample-size 20 --noplot
   cargo bench --bench sqlite_compare -- compare_full_scan --measurement-time 2 --warm-up-time 0.5 --sample-size 20 --noplot
   ```
   每段 1~3 分钟；插入段 SQLite 侧每轮 100 插 + DELETE，最慢。
2. **精确均值提取**（stdout 有缓冲延迟，数字从落盘 JSON 取最稳）：
   ```bash
   python3 - <<'PY'
   import json, glob
   for d in sorted(glob.glob('target/criterion/compare_*/*/new')):
       e = json.load(open(d + '/estimates.json'))
       print(d.split('/criterion/')[1].rsplit('/new',1)[0], round(e['mean']['point_estimate'],1), 'ns')
   PY
   ```
3. **CLI 级负载生成**（行数受单参数上限约束，见"失败处理"1；2000 行 ≈103KB 安全）：
   ```bash
   python3 - <<'PY'
   rows = 2000
   with open('/tmp/workload.sql','w') as f:
       f.write("CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER);\n")
       for i in range(rows): f.write(f"INSERT INTO bench VALUES ({i}, 'user_{i}', {i*10});\n")
       f.write("SELECT COUNT(*) FROM bench;\n")
   PY
   ```
4. **一次性加载对比**（各自干净起跑，先 `rm` 库文件）：
   ```bash
   /usr/bin/time -v ./target/release/rtsql /tmp/r2k.db "$(cat /tmp/workload.sql)" >/dev/null 2>/tmp/t_r.txt
   grep -E "Elapsed|Maximum resident" /tmp/t_r.txt
   /usr/bin/time -v sqlite3 /tmp/s2k.db < /tmp/workload.sql >/dev/null 2>/tmp/t_s.txt
   grep -E "Elapsed|Maximum resident" /tmp/t_s.txt
   stat -c "%n %s" /tmp/r2k.db /tmp/s2k.db   # 关闭后主库文件体积（rtsql 经 close checkpoint）
   ```
5. **one-shot 时延**（50 次均值）：
   ```bash
   S=$(date +%s%N); for i in $(seq 50); do ./target/release/rtsql /tmp/r2k.db "SELECT 1" >/dev/null; done; E=$(date +%s%N)
   awk "BEGIN{printf \"rtsql: %.2f ms/run\n\", ($E-$S)/50000000}"
   # sqlite3 同型循环，除数相同
   ```
6. **二进制体积**：`stat -c %s` 对 `target/release/rtsql` 与 `$(command -v sqlite3)`。

## 验证

- criterion 每段出现 `Collecting 20 samples` 与 `time: [lo mean hi]` 区间；`estimates.json` 各条目有数值。
- 加载段：两引擎 `Elapsed` 与 `Maximum resident set size` 均有数；两个库文件 stat 有值（2026-09-24 实测：rtsql 2k 加载 3.21s / RSS 17124KB / 主文件 307264B；sqlite 5.29s / 4156KB / 49152B）。
- 时延段循环完整跑完（实测 rtsql 10.77ms/run vs sqlite3 1.15ms/run，50 次）。
- 引擎级（实测均值）：insert100 4.80ms vs 251.25ms；pk lookup 1.57µs vs 6.80µs；full scan 296.6µs vs 97.9µs。
- 结论落点：数字写入双语 README 对比板块并附口径与"单机观察"限定。

## 失败处理

1. **`Argument list too long`（E2BIG，time -v 下 rtsql 静默失败、time_rtsql.txt 首行报错）**：SQL 整体作为单个 CLI 参数超过内核 `MAX_ARG_STRLEN`（≈128KB）。停止该路径，把行数降到 ~2000（≤105KB）或分多次调用；不要用 `xargs` 硬塞。
2. **测量被并发实例污染**：启动前 `ps aux | grep "[r]un_all\|[r]un_v2"` 确认唯一实例；日志出现重复段落/交错即废弃重跑（本流程首次运行曾因旧实例存活导致双跑交错）。
3. **`pkill -f` 自匹配**：清理后台实例时 `pkill -f "[r]un_all"` 括号防自匹配，否则会杀死当前工具 shell。
4. **`Gnuplot not found`**：无害，加 `--noplot` 消除。
5. **criterion `Warning: Unable to complete 20 samples in 2.0s`**：慢基准的正常提示（样本数已保证），可不理会或加大 `--measurement-time`。
6. **sqlite3 加载显著偏慢（5k 行 12.76s）**：默认回滚日志逐语句 fsync 所致，是口径内的预期现象，不是故障。
7. **变量丢失导致 `cannot run : No such file or directory`**：`time -v "$BIN"` 的 BIN 为空——命令里显式写绝对路径，不依赖跨命令的 shell 变量。

## 回滚

只读测量：产物仅落在 `/tmp`（工作负载、临时库、time 输出）与 `target/criterion/`（构建产物区，Git 忽略）。回滚 = `rm` 对应临时文件；不修改仓库源码、系统安装或用户数据。不可逆点：无。

## 证据

- 本会话执行记录（2026-09-24，revision `1a2d877` 工作树）：criterion 三段、`estimates.json` 提取、time -v 负载、stat、50 次时延循环的命令与原生输出；失败样例（E2BIG 报错原文、双实例交错日志）同场保留于会话记录。
- `/tmp/opencode/compare/run2.log`（v2 干净单实例全量日志；/tmp 临时，重启即失）。
- 持久化结果：`README.md` / `README.zh-CN.md`「性能与资源对比」板块（与本 Runbook 同批提交）。
- `target/criterion/compare_*/.../estimates.json`（本机可复查）。
- 限制：全部数值为该机器当日观察；SQLite 版本、内核参数（`MAX_ARG_STRLEN`）、CPU 拓扑换环境须重测。
