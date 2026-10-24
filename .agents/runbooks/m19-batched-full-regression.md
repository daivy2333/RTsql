# M19 分批全量回归执行配方

- Status: active
- Last validated: 2026-10-23
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/` 集成测试套件（130 个测试目标）
- Source: change `2026-10-23-ms33-drop-persistence-failure` Iteration 000 / Cycle 000 Act Response（`openspec/changes/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`，2026-10-23）

## 适用范围

- 需要给出「全量回归通过」结论时（公共规则 M19 强制分批：一次无目标过滤的 `cargo test` 不作为全量结论）。
- 验证某次改动没有破坏既有行为，需要每批的 passed/failed 计数作为交付证据。
- 不适用：单次定向验证（`cargo test --test <name>` 可用默认并行度，M19 明确不约束）；单测或 bench 跑太久/假死的诊断（见 `.agents/runbooks/test-bench-diagnosis.md`）；基准 baseline 对比（见 `.agents/runbooks/ms08-bench-comparison.md`）。

## 前置条件

- 工作区可编译；`cargo test` 能构建本项目的测试目标（130 个 `tests/*.rs` + `--lib`/`--bins`）。
- 机器可承受串行执行 14 批；分批的目的是避免一次调用构建链接全部测试二进制并长时间独占机器。
- 全程只跑一批：批内 `--test-threads=1`，批与批顺序执行，不要并发启动多批。

## 操作步骤

1. 先跑库与二进制目标一批：

```bash
cargo test --lib --bins -- --test-threads=1
```

2. 再把 `tests/*.rs` 按**文件名字典序**排序，每 10 个文件一批，每批一次独立调用：

```bash
ls tests/*.rs | sort
# 第 N 批（N 从 1 起）：
cargo test --test <f1> --test <f2> ... --test <f10> -- --test-threads=1
```

3. 每批记录退出码与 passed/failed/ignored 计数。计数可从 cargo 原生输出用一步管道读出，管道只做计数、不做判定：

```bash
cargo test --test <f1> ... -- --test-threads=1 2>&1 | grep -E "^test result:"
```

4. 全量结论等价于「每一批各自 0 failed」。缺一批不能声明全量通过；逐批计数要保留在交付记录里。

## 验证

- 成功判据：每一批的 cargo 退出码为 0，且各批 `test result:` 行的 failed 计数为 0。判定只看 cargo 自己的退出码与原生输出，不新增判定脚本、封装命令或判定层。
- 2026-10-23 实例（`acfd274` + MS33 的 D1 改动，130 个测试目标）：14 批，合计 **2151 passed / 0 failed / 2 ignored**。2 个 ignored 是 `cli_test.rs` 里长期标 `#[ignore]` 的校准/诊断用例（`calibration_recovery_time`、`diagnostic_wal_parse`），与本次改动无关。与 MS32 收尾基线 2144/0/2 相比正好多出本次新增的 7 个用例。
- 批次规模经验值 8–12 个目标/批（M19）。测试目标数变化后按同一分组规则重算批数。

## 失败处理

- **某批 failed**：先单独复核该用例，再整批复核一次（失败后判定是否偶发错误的一次复核；复核通过按通过处理，并在验证记录注明首跑失败）。
- **cargo 在首个失败目标后中止该批的后续目标**：批内目标按给定顺序执行，第一个失败目标之后的测试二进制**不会运行**。复核时必须重跑整批，否则会把「未执行」误记成「通过」。2026-10-23 的 B04 首跑即此形态：e2e_test 失败后 encryption_test / executor_test / explain_test 未执行，整批复核才补齐这 56 个用例。
- **某批跑很久**：见 `.agents/runbooks/test-bench-diagnosis.md`（死锁/空转/setup 过重的区分与 `ps aux` 看 CPU% 的做法）。
- **失败测试与本次改动无关**：不改其他模块去迁就全量。按当前 change 的纪律把失败测试名写进 Act Response 并停止，由用户或 Plan 决定后续。
- 判定失败归因不明时不要重复运行整轮全量；按公共规则只对疑点做定向审计。

## 回滚

不适用（验证执行方法，无产品状态变更）。分批过程不修改仓库文件；测试自身产生的库文件与临时目录随用例结束清理。

## 证据

- `openspec/changes/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`（Act Response「验证命令与实际结果」的 1.4 分批明细表与 B04 首跑失败/复核记录）
- 公共规则 M19：`.agents/memory/project-model.md` 的「M19: 全量测试分批执行约束」
- 相关配方：`.agents/runbooks/test-bench-diagnosis.md`（慢/假死诊断）、`.agents/runbooks/in-process-reopen-testing.md`（进程内 close→reopen 测试夹具）
