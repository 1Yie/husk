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

## 第三轮

- **`agent-tools-shell` 的 `SERVICES` 已按会话隔离**：新增 `ServiceRegistry` / `ServiceRegistrySlot` /
  `install_registry`；`husk-host` 在每个会话的 `ToolCtx` 上安装一个，委派的子上下文共享它，会话丢弃时服务随之被杀。
  没装槽位的上下文仍用进程默认注册表（行为不变）。测试 `services_are_scoped_to_the_owning_registry` 覆盖。
- **`scripts/export-kernel.sh <dir>`**：把内核闭包（ipc / llm-core / tool / kernel / testkit）导出成独立 cargo 工作区，
  并带上 `check-kernel-deps.sh`。导出树在没有产品代码的情况下 `cargo test --workspace` 全绿，闭包校验通过。

## 有意保留的进程级全局

- `husk-host::sandbox_prefs::LIMITS`：用户级偏好，设置页改动要对所有活跃会话即时生效，进程级就是它的语义；
  拆成按会话持有会改变这一点，还要改三个前端入口。`SwitchableSandbox::with_limits` 已允许宿主注入自己的来源。
- `agent-store::shared_db`：进程内唯一的会话库，数据目录由 `init_data_dir` 一次设定。
- `agent-tools-serena::MANAGERS`：按工作区根目录共享昂贵的 Serena 服务，`shutdown(root)` 负责释放。
- `agent-kernel` 的 `ACTIVE_STEERS`、`todo` 的 `LOCKS`：分别按 session / store key 隔离。

这些都只在「两个产品同进程」时才可能冲突，且不在内核闭包内。

## 仍未做

- `wrap_spawn` 的 bwrap 遮罩竞态窗口（长驻服务启动）。
- 真正搬到独立仓库、path 依赖改 git/registry：需要你决定仓库位置与发布方式。

## 第四轮：拆成三个并列仓库

```
workspace/
  korei/      内核：agent-ipc · agent-llm-core · agent-tool · agent-kernel · agent-testkit
  korei-ext/  扩展：agent-capabilities · agent-sandbox · agent-computer · agent-plugin · agent-paths
              · agent-tools-{fs,web,shell,computer,office,serena}
  agent-rs/   产品侧（Husk 暂不拆）：agent-llm · agent-coding · agent-store · agent-context
              · agent-skills · husk-host · app-*
```

- 依赖方向：`agent-rs → korei-ext → korei`；`agent-rs → korei`。内核不认识扩展，扩展只依赖内核的契约
  （`agent-tool`、`agent-llm-core`），不依赖 `agent-kernel`。
- 目前是兄弟目录 path 依赖；扩展仓库把内核 crate 集中声明在 `[workspace.dependencies]`，改成 git/registry 只需改一处。
- 各仓库自带依赖校验：`korei/scripts/check-kernel-deps.sh`、`korei-ext/scripts/check-deps.sh`、
  `agent-rs/scripts/check-deps.sh`（只管留下来的 crate）。
- 迁移中暴露的一处耦合：`agent-llm/tests/compat_wiring.rs` 读内核源码，现改为读 `../../../korei`，
  并在目录缺失时给出明确报错。
- 验证：`korei`、`korei-ext`、`agent-rs` 各自 `cargo test --workspace` 全绿。

### 待定
- 内核仓库的远端地址（`korei-ext/.github/workflows/ci.yml` 里是 `OWNER/korei` 占位）。
- 三个仓库的 CI 彼此依赖兄弟目录，远端化之前 CI 跑不通。
- 新仓库的提交历史是一次性导入，没有保留原 crate 的 git 历史。


## 第五轮：真实依赖

`korei` 与 `korei-ext` 已建为 GitHub 私有仓库（`1Yie/korei`、`1Yie/korei-ext`），三个仓库之间改为 git 依赖：

- `agent-rs` 与 `korei-ext` 在 `[workspace.dependencies]` 里集中声明 `git = "ssh://git@github.com/1Yie/korei*.git", branch = "main"`。
- 各仓库 `.cargo/config.toml` 设 `net.git-fetch-with-cli = true`，私有仓库走系统 git 与 SSH 配置。
- 验证：把本地 `korei` / `korei-ext` 目录挪走后，`agent-rs` 仍能编译并通过测试。
- `compat_wiring` 改为用 `cargo metadata` 找依赖源码，不再假设兄弟目录布局。
- CI 需要仓库密钥 `KOREI_TOKEN`（能读取 korei / korei-ext 的 token），未配置前 CI 无法拉取私有依赖。
