# agent 运行内核抽离总结

## 内核闭包（Korei）

`agent-ipc`、`agent-llm-core`、`agent-tool`、`agent-kernel`（测试支撑 `agent-testkit` 仅作 dev 依赖）。
`cargo tree -p agent-kernel -e normal` 中已无 rig / reqwest / keyring / toml。

## 本轮改动

1. **goal/plan 控制器**：内核内已无相关代码，只在 `agent-coding`（`goal.rs`/`plan.rs`/`policy.rs`）。
   内核通过 `TurnPolicy`/`ToolPolicy`/`Verdict` 接缝承载，`engine.rs` 里只剩注释措辞。无需再拆。
2. **拆分 `agent-llm`**：
   - `agent-llm-core`（契约）：`types`、`provider`（`LlmProvider`）、`sampler`、`masking`、
     `compat`（`ProviderCompat`）、`thinking`（thinking 等级规则）。
   - `agent-llm`（实现）：`config`、`factory`、`rig_bridge`，依赖 rig-core、reqwest、keyring、toml。
   - `agent-llm` 以 `pub use` 重导出 core 的模块，产品侧的路径不变。
   - 仅用契约的 crate 改依赖 core：tool、kernel、testkit、store、plugin、tools-web、tools-computer、coding。
     husk-host 和各前端仍用完整 `agent-llm`。
   - `scripts/check-deps.sh` 白名单已更新。
3. **遗留问题**：
   - `husk-host` 两个 warning：删除未用导入；`remember_memory_forget_roundtrip` 缺 `#[tokio::test]`，
     一直没被运行，补上后通过。
   - `cargo fmt --check` 已清零（全工作区 `cargo fmt --all`）。
   - `agent-plugin` hooks 偶发失败：在 `hooks.rs` 的 spawn 处对 `ETXTBSY` 做有限重试。

## 验证

`cargo check --workspace --all-targets`、`cargo test --workspace`（连续两遍）、`standalone_assembly`、
`scripts/check-deps.sh`、`cargo fmt --check` 均通过。

## 未完成 / 需注意

- `agent-plugin` 偶发失败我只见到一次，没抓到原始报错，ETXTBSY 是依据测试写法做的推断；
  修复后两遍全量均未复现，但不能证明已根治。
- `agent-sandbox` 的偶发失败本轮没有出现，未做处理。
- 进程级全局（`agent-store::shared_db` 等）不在内核闭包内，未处理。
- 抽成独立仓库（path 依赖改 git/registry、semver 约束）未做。
- 工作区原有大量未提交改动，本轮改动与之混在一起，尚未提交。
