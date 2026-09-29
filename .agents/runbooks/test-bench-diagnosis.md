# 测试与基准诊断配方（测试过长/假死/criterion 基线）

- Status: active
- Last validated: 2026-09-24
- Environment: Linux x86_64、Rust/Cargo、criterion.rs；RTsql 测试套件（`tests/` 集成 + `#[cfg(test)]` 单元）与 `benches/`（criterion）
- Source: 2026-09-24 knowledge spec 退役迁移（原 K20 测试诊断框架 / K21 baseline 纪律 / K34 tempdir leak / K35 bench 技巧集，按原编号保留小节）；完整原文与退役映射见清理 carrier（ARC-202609241843b，经 `openspec/specs/knowledge/spec.md` 底部 arc 指引定位）；基准对比方法另见 R17 `ms08-bench-comparison.md`

## 适用范围

- `cargo test` 单个测试或全量运行远超秒级、疑似死锁/空转/setup 过重的诊断。
- criterion bench 报 sample 超时、target time 警告、或性能优化前后无 baseline 可比的补救与预防。
- 不适用：产品运行期故障（走 issues/analysis）；benchmark 数值的跨引擎对比（见 R26）。

## 诊断路径（原 K20）

核心断言：Rust 测试或 bench 在 RTsql 项目**正常应在秒级完成**。超过 30s 几乎都是配置或 bug 引起。

| 症状 | 诊断命令 | 根因 | 修复 |
|------|----------|------|------|
| 跑很久无输出 stdout 0 字节 | `ps aux \| grep <test_or_bench>` 看 CPU% | (a) 无输出空转 (b) 真的死锁 | 加 eprintln! + `cargo test -- --nocapture` |
| 单个 test 跑 > 30s | `cargo test <name> -- --nocapture` | 死锁/无限循环/setup 卡死 | 拆测试 + 检查锁 + 检查 for 条件 |
| bench criterion 报 "Unable to complete 100 samples in Xs" | 看 X（target time）| setup 混进 iter 闭包 | 一次构建 dataset 在 bench_with_input 之前 |
| bench 警告 "increase target time to 1456s" | criterion 自动算的目标时间 | 100 samples × 1.5s/iter = 150s/规模 | `--sample-size 10 --measurement-time 2` 快速验证 |
| cargo test 全绿但慢 | `cargo test --release` 看加速比 | debug 模式无优化 | 日常 debug 跑，生产 release 验证 |

三类空转的区分：

1. **死锁（CPU ~0%）**: 锁循环等待，进程几乎不耗 CPU。`ps aux`: CPU% < 5% 持续。
2. **无限循环（CPU 100%）**: 条件永不满足。`ps aux`: CPU% ~100% 持续。
3. **Setup 过重（CPU 中高但每步慢）**: 每个测试都在重建。

关键 cargo 命令：

```bash
# 诊断 hang：
cargo test <name> -- --nocapture 2>&1 | tee /tmp/test.log &
ps aux | grep cargo    # 看 CPU%
kill -9 <pid>          # 必要时 kill

# 诊断 bench 过慢：
cargo bench <name> -- --sample-size 10 --measurement-time 2
# 如果还是慢 → setup 问题；快 → 调大 sample-size
```

## 基线纪律（原 K21）

性能优化前必须 `cargo bench --save-baseline before-X`，否则无 before 参照无法验证目标。

反例（M36 教训）：实施时才跑 baseline，已是实施后状态；`git stash` 与 master 上其它改动冲突，无法干净还原。

正确流程：

1. 实施前：`cargo bench --save-baseline before-X`
2. 实施
3. 实施后：`cargo bench -- --baseline before-X` 对比

## 预防检查清单（提交新 test/bench 前）

- [ ] 单个 test 跑 < 5s（无 I/O 的纯逻辑 < 100ms）
- [ ] 大数据集 bench 先用 `--sample-size 10 --measurement-time 2` 验证 setup
- [ ] 怀疑死锁时 `ps aux` 看 CPU%，加 eprintln! + `-- --nocapture`
- [ ] 数据规模从 1K 起步，不要 100K 起跑
- [ ] 复用 tempfile 不持锁到测试结束

## bench 技巧集（原 K35 / K34）

- 技巧 1: 共享 tokio::runtime——避免 per-iteration 创建 runtime。
- 技巧 2: RTsqlDirect in-process——直接调用 API，避免 network overhead。
- 技巧 3: criterion Throughput——设置 throughput 更准确测量。
- 技巧 4: `#[inline(never)] + std::hint::black_box`——防止编译器消除 fetch_add 真实开销。
- 技巧 5: `std::thread::spawn + Arc` 共享计数器——多线程争用基准（避免 rayon 依赖）。
- tempdir leak 模式（原 K34）: 独立 WAL 层 benchmark（不经过 SQL 层）需要文件在 bench 期间存活时，`std::mem::forget(dir)` 阻止 tempdir drop 删除——先例 `benches/wal_group_commit_bench.rs`。

## 失败处理

- 判定一律用 cargo/criterion 自身退出码与原生输出；不新增判定脚本（CLAUDE.md 验证纪律）。
- baseline 缺失且改动已落地：不伪造 before 数据——以改动后数据 + 机制归因记录，标注「无 before 基线」（K18 先例），或回退改动重走流程。

## 回滚

不适用（诊断与基准方法，无产品状态变更）。

## 证据

- 原始条目：knowledge spec K20/K21/K34/K35（2026-09-24 退役，全文见清理 carrier）
- 历史见证：`benches/tx_id_bench.rs`、`benches/data_scan_bench.rs`（K19 踩坑修复先例）、`benches/wal_group_commit_bench.rs`（K34 先例）、归档 change 内 M36/K18 教训记录
