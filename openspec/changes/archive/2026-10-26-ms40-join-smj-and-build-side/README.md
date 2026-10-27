# 2026-10-26-ms40-join-smj-and-build-side

排序合并连接，以及等值哈希连接在时间门通过后选择行数较少的一侧构建。触发条件已收窄：两侧都是基表扫描，且每个连接键都是该表写出的整数 `PRIMARY KEY`。隐式首列整数键仍走哈希。计划期选择已经接受。`iterations/002-signed-zero-match/000-initial.md` 的 Plan Context 保持 ready，那一轮只归一哈希连接交给哈希表的键。`iterations/003-current-catalog-row-length/000-initial.md` 的 Plan Context 为 ready：只把两条当前代目录行长度断言改到已接受的尾字节，不改序列化。
