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

## 第二轮（提交后继续）

- **sandbox 偶发失败（已定位并修复）**：不是代理就绪竞态。并行测试在 `$HOME/.config` 下创建临时 socket，
  `$HOME` 扫描之后它被删除，bwrap 创建遮罩挂载点失败（`Can't mkdir parents`）。
  `LinuxBwrap::run_command` 现在识别该失败并重建 argv 重试（最多 3 次）。加压下 6 轮全绿（修复前 3 轮中 2 轮失败）。
  `wrap_spawn` 返回未启动的 `Command`，没有同样的重试点，该窗口仍在（仅影响长驻服务启动）。
- **内核内的进程级全局**：`todo` 工具的内存兜底原来按 `ToolCtx` 地址存进进程表（地址复用会串数据，且不释放），
  现在挂在 `ToolCtx::todo_fallback` 上随上下文释放。`delegate.rs` 的 `ACTIVE_STEERS` 与 `todo.rs` 的 `LOCKS`
  仍是进程级，但按 session / store key 隔离，行为是对的，没有改。
- **接缝 semver 冻结**：`crates/agent-kernel/tests/seam_api.rs` 只用公开 API 实现 `TurnPolicy`、`PluginRouter`、
  `SubagentHost`、`TodoStorage`、`BatchProfile`；接缝变形会在这里先编译失败。
- **内核闭包依赖校验**：`scripts/check-kernel-deps.sh`（已加入 CI），不依赖产品表，
  断言闭包内部依赖，并禁止 rig-core / reqwest / keyring / toml 进入内核依赖树。
- **发布元数据**：内核四个 crate 的内部依赖补了 `version`。`agent-ipc`、`agent-llm-core` 已能 `cargo package`；
  `agent-tool`、`agent-kernel` 要等上游先发布到 registry 才能打包，这是预期。

## 仍未做

- 产品侧进程级全局（`agent-store::shared_db`、`husk-host::sandbox_prefs::LIMITS`、
  `agent-tools-shell::SERVICES`、`agent-tools-serena::MANAGERS`）：只在两个产品同进程时才冲突，且需要改宿主装配，没有动。
- 真正搬到独立仓库、path 依赖改 git/registry：需要你决定仓库位置与发布方式。
