# MS08 bench 基线采集与前后对比判读

- Status: active
- Last validated: 2026-09-05
- Environment: WSL2（Linux x86_64, microsoft-standard-WSL2）、Rust 2021 / Cargo、criterion 0.5、strace 5.16（/usr/bin/strace）
- Source: change `2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch`（`iterations/000-pread-pwrite/000-initial.md` Act Response、`iterations/001-prefetch/000-initial.md` Act Response、`iterations/001-prefetch/001-replan.md` Plan Review）；commits `dac6783` / `709c85d`

## 适用范围

- MS08 各 T（T03-T06 及后续同类性能优化）实施前后的量化对比：criterion 基线落盘、strace syscall 计数对比、before/after bench 判读。
- 不适用：功能正确性验证（用 `cargo test`）；非 criterion 框架的计时；单次运行无对照组的结论。

## 前置条件

- 基线采集时工作树代码必须处于目标"改前"状态：`cargo bench` 从工作树编译 lib + benches，未提交的产品代码改动会进入测量（测试文件不参与 bench 编译，可与采集并行）。
- strace 可用；测量对象为一个页 I/O 密集的既有测试二进制（如 `--test storage_test` 对应的 `target/debug/deps/` 下二进制）。
- 已知 WSL2 会话间环境漂移 ±1~5%（对照组实测），ms 级 bench 条目噪声带 ±5~18%——所有判读必须走对照组，不接受裸 p 值。

## 操作步骤

1. **before 基线（实施前，顺序执行）**
   1. `cargo test --all`——确认基线全绿。
   2. `cargo bench --bench <各bench> -- --save-baseline before-MS08-T<n>`
      - 陷阱：criterion 参数必须经 `--` 透传。cargo 直接拒绝裸 `--save-baseline`（`unexpected argument '--save-baseline'`）。
   3. strace before：`strace -f -c -e trace=lseek,pread64,pwrite64,read,write <测试二进制>`
      - 陷阱：x86_64 strace 5.16 无 `pread`/`pwrite` syscall 名（直接报错），只用 `pread64`/`pwrite64`。
2. **实施后对比（同一二进制、同一 trace 集合、同一 bench 套件）**
   1. `cargo test --all` 全绿。
   2. `cargo bench --bench <各bench> -- --baseline before-MS08-T<n>`
      - 严禁误加 `--save-baseline`——会用新数据覆盖命名基线，前后对比失效。
   3. strace after 同口径重跑，对比计数表。
3. **退出码**：命令接管道时先 `set -o pipefail`，否则 cargo 失败会被 grep 掩盖。
4. **判读**：按"失败处理"一节的三步法执行。

## 验证

- 基线落盘判据：`target/criterion/**/before-MS08-T<n>/` 目录出现且条目数符合预期（T01 实测 18 条目 = micro 11 + data_scan 4 + buffer_pool_concurrency 3；T02 实测 4 条目 = data_scan_bench 4）。
- strace 判据：before/after 计数可读且差值可归因（T01 实测：lseek 33→3、pread64 4→26、pwrite64 0→8——页路径 2 syscall→1 结构变化）。
- bench 判据：`--baseline` 输出每条目 change 区间 + p 值；结论（改善 / 未达预期 / 环境漂移）有对照组支撑。
- 全部命令退出码 0。

## 失败处理（判读三步法）

1. **对照组判定**：取同 bench 套件中未被本改动触碰的路径作环境对照（如 `data_scan_bench` 的 `scan_via_index` 用 `ScanExecutor`，不经过被测执行器）。对照 p>0.05 → 环境稳定，被测路径 p<0.05 可归因代码；对照也 p<0.05 → 会话环境漂移，被测路径同量级变化不可归因代码。
2. **p 值与方向**：criterion 的 p<0.05 只在对照组稳定时采信；方向（改善/回退）与幅度须与既有机制证据一致。
3. **机制归因**：量化解释须对齐数据规模与缓存容量（实测基准：池容量 100 页时 1000 行 ≈17 页全暖缓存、稳态零 miss；10000 行 ≈164 页部分冷），并对齐成本模型（跨线程 spawn/wake 任务生命周期 ≈4µs/页 vs WSL2 OS 页缓存 4KB pread64 ≈2-3µs——暖缓存下"隐藏加载延迟"类设计只剩纯开销）。
4. **strace 局限**：对"时序型"变化无判别力——只改 I/O 时机不改次数的改动（如预取）pread64 计数不变，此时以 criterion + 对照组为决定性证据。
5. **复现要求**：边缘判读必须跨轮复现才采信。实例（T5.4）：同一 diff 两轮测得 -3.4~-3.8%（p=0.00），第三轮独立复跑 +0.28%（p=0.73）未复现 → 判会话环境漂移，不作结论。
6. **停止条件**：被测路径显著回退且机制无法归因 → 停止扩测，按流程返回 Plan；判读是决策辅助，不是修复手段。

## 回滚

本操作只读：bench/strace 不修改仓库文件（criterion 数据落在 `target/criterion` 构建产物区）。无需回滚。唯一不可逆点：误加 `--save-baseline` 覆盖命名基线——恢复方式只能重采基线，且必须回到改前代码状态；预防优先。

## 证据

- `openspec/changes/archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/iterations/000-pread-pwrite/000-initial.md`：T1.2/T1.3/T3.2/T3.3 验证表（基线 18 条目、strace before/after、bench 对比结论）、Deviations 1-3（`--` 透传、strace syscall 名、clippy `--`）。
- `.../iterations/001-prefetch/000-initial.md`：默认路径回退 +40~47%/+17~18%（p<0.05）、对照组 scan_via_index 无变化、机制归因、strace 对时序型变化无判别力的论证。
- `.../iterations/001-prefetch/001-replan.md`：根因诊断四点（零阻塞对象配置 / syscall 对照 / 加性成本模型 / 环境边界）+ Plan Review 第三轮未复现记录（p=0.24/0.73）。
- 限制：所有数值（噪声带、µs 成本、页数）为 WSL2 本环境 2026-09-05 实测，换环境须重测；strace 行为与版本相关（5.16）。
