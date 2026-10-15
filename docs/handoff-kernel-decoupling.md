# 接力：agent-kernel 解耦（阶段 1）

> **进度更新（阶段 1 已收尾）**：本文件的 T1–T8、T10、T11 全部完成，T7 的五个工具包也已抽出。
> 唯一剩下的是 **T9（宿主代码外移）**，其接力文档见 **`docs/handoff-t9-host-extraction.md`**
> ——那里有实测的模块依赖、建议切片、已确立的接缝模式、验证纪律与已知陷阱。
> 本文件保留作历史记录；它的 §1 提交表与 §9 进度行已更新到最新。

目标：下一个 Agent 产品（如 AI SSH 工具）要能只挑需要的模块复用。kernel 不能内置
“沙盒必有、桌面必有、Coding 语义”。判断标准：**产品 2 只依赖 kernel/tool/llm、
零改动 kernel 即可装配**。

## 1. 已提交（本地 main，未推送）

| 提交 | 内容 |
|---|---|
| `a5502e5d` | agent-llm：egress masker、降级链、Anthropic adaptive thinking、配置写回 |
| `d0736e1c` | agent-kernel：masker / 降级链接入会话 |
| `abbb41e5` | app-tauri：provider 设置编辑器、配置保存校验 |
| `2b40ed50` | ci：`scripts/check-deps.sh` 依赖律 + `.github/workflows/ci.yml`；`.pi/` 入 `.gitignore` |
| `48c9f28e` | refactor(kernel)：goal / plan controller 迁入 `ToolCtx` 扩展表（试点） |
| `f031f796` | refactor(kernel)：desktop 后端改为可选宿主能力 |
| `4fa822ee` | refactor(kernel)：沙盒改为扩展表能力（`SandboxSlot`/`SandboxLimitsSlot`） |
| `8e0e4e8d` | T3+T4：`SwitchableSandbox::with_limits` 注入开关；skill 挂载由宿主注入 |
| `cbd29f5d` | T5：`ask` 迁入扩展槽 |
| `3b4d7377` | T5：`subagent` 迁入扩展槽 |
| `42fd0e05` | T5：`memory` 迁入扩展槽 |
| `75aa3523` | T5：`session` 迁入扩展槽 |
| `d77b14cc` | T5：`plugins` 迁入扩展槽 |
| `cdafbc3b` | test：锁定 goal 催促上限（8 次） |
| `7ac38039` | test：锁定 goal 催促先于 blueprint 检查 |
| `1b820f17` | T6：收尾分支 plan/goal/blueprint 迁入 `TurnPolicy` |
| `6d90c93d` | T6：computer 限额迁入 `TurnPolicy::before_dispatch` |
| `648af4c8` | T6：分阶段写入集合收敛为 `registry::is_staged_write` |
| `71f820f3` | T7 前置：`ui_tx` 迁入扩展槽（`ToolCtx` 已无产品类型字段） |
| `292895ea` `d5fd8a26` | docs：修正我此前引入的两处文档注释错位（`ask`/`resolve`） |
| `7951c03c` | T7 前置：`registry.rs` 拆为契约 + `host_slots.rs` + `builtins.rs` |
| `b611f126` | T7：抽 `agent-tool` crate（方案 A，契约只依赖 `agent-llm`） |
| `f1da0164` | T8（部分）：抽 `agent-skills` crate（零 workspace 依赖） |
| `d935a8c5` | T10（部分）：新增 `agent-paths`；`agent-plugin` 与插件会话路径改由宿主注入 |
| `5c157601` | T10（部分）：`AppPaths` 贯穿 `SessionConfig`；`session.rs` 的 AGENTS.md 路径已去 `husk` |
| `2f2d5236` | T10（部分）：`ToolCtx` 增 `AppPaths` 槽；`delegate`/`screenshot` 不再命名产品 |
| `cf84968d` | T10（部分）：`agent-llm` 配置目录改为参数；`agent-plugin` 的 MCP 握手用 `app_name()` |
| `7703c60c` | T10（部分）：`session_store` 数据目录由宿主设定；`HUSK_SESSION_DB`/`husk-test-` 改名 |
| `794318a9` | T10：内核最后三处去 `husk`（UA / 沙箱临时目录 / serena clientInfo）|
| `3aedda3c` | T8：`session_store` 拆出 `prefs.rs`（仓库不再命名偏好类型）|
| `993e87eb` | T8：store 暴露公开的全局行 API；临时放宽的可见性全部收回 |
| `1fe858de` | T8 ✅：抽出 `agent-store`（只依赖 `agent-llm`）|
| `fd21edff` | 清理：删掉 `cf84968d` 遗留的未使用测试辅助 `test_paths`（agent-plugin 警告）|
| `c4617876` | T11：独立 crate `minimal-agent`（只依赖 kernel + llm）；修复接缝泄漏——kernel 转发 `HunkTracker` |
| `1587ed9e` | T11：第三个工具；断言模型实际收到的 schema 只含产品自己的工具 |
| `99b30a05` | 清理：删除抽取 `agent-store` 时漏删的 `session_store/{files,migration}.rs` 死副本 |
| `e92659c8` | T7：抽出 `agent-tools-fs`（6 个文件工具）；契约新增 `WorkspaceCtx` |
| `50ad59fc` | T7：抽出 `agent-tools-web`；内核不再依赖 `reqwest`/`htmd` |
| `e39f0945` | T7：抽出 `agent-tools-shell`；`SandboxLimits` 移入 `agent-sandbox`；契约新增 `SandboxCtx` |
| `e6c3ec22` | T7：抽出 `agent-tools-computer`；契约新增 `DesktopCtx`；修掉 `app_paths` 被绕过的回归 |
| `f68051c7` | T7 ✅：抽出 `agent-tools-office`；五个工具包全部完成 |

基线：`f031f796` 时 336 通过。当前 `cargo test -p agent-kernel` 为 **349 通过 / 0 失败**。
依赖律 `./scripts/check-deps.sh` 通过，下游 `cargo check -p app-cli -p app-tui -p husk --tests` 0 error。

## 2. 扩展表机制（已就绪）

- `tools/registry.rs` 的 `Extensions`：按 `TypeId` 键控，`Arc` 共享，`get::<T>()` 返回 `Option<Arc<T>>`。
- 新类型键：`DesktopSlot`、`SandboxSlot`、`SandboxLimitsSlot`（newtype，避免 trait object 类型冲突）。
- 访问器：`ctx.require::<T>(what)`、`ctx.desktop()`、`ctx.sandbox()`、`ctx.sandbox_limits()`，缺失时返回 `ToolError::Failed`，不 panic。
- 宿主安装：`with_desktop` / `with_sandbox` / `install_sandbox` / `with_sandbox_limits`。
- 子代理继承：`inherit_desktop` / `inherit_sandbox`，**只复制指定槽位，不整体复制 `ext`**，否则 goal/plan controller 会被共享。
- `ToolCtx::new` 不再探测硬件（不装 desktop、不装 sandbox）。

## 3. 工作区状态

**干净**（仅未跟踪 `docs/` 本身）。沙盒迁移与 T3–T6 已全部提交。
`cargo test -p agent-kernel` = **349 passed / 0 failed**。

## 4. 已知风险 / 待办（按优先级）

**已完成**：原 1–3、6、8 项。沙盒迁移全量验证通过；`service_tool` 不再改全局开关；
`global_skill_roots()` 改由宿主注入；`ToolCtx` 的产品字段（`ask`/`subagent`/`memory`/
`session`/`plugins`）已全部迁入扩展表。

**仍待处理**：

1. **宿主仍用全局 `sandbox_prefs::set`**（`app-cli`、`app-tui`、`app-tauri/.../sessions.rs`、
   `session_manager.rs`）。过渡状态：cli/tui/tauri 走 `current()` → 改 → `set()`，依赖
   `session_manager` 的种子已写入，否则 `extra_ro_mounts` 会丢。终态改为宿主直接
   `with_sandbox_limits`，删全局（T9）。
2. **`session.rs` / `session_manager.rs` 仍在 kernel 内**，是宿主代码。T9 计划移到
   `husk-host`，`SessionActor` 的沙盒装配随之迁出。
3. **`registry.rs` 仍内联产品类型的槽与方法**，是 T7 拆 `agent-tool` 的剩余前置：
   - 槽定义：`UiSlot`(`channels::UiSink`)、`SandboxLimitsSlot`(`sandbox_prefs`)、
     `SubagentSlot`(`tools::delegate`)、`SessionSlot`(`session_store`)、`PluginSlot`(`engine`)。
   - 方法：`ui()/install_ui`、`subagent()/install_subagent`、`session()/with_session`、
     `plugins()/with_plugins`、`sandbox_limits()/with_sandbox_limits`、
     `with_controllers()`（goal/plan），以及 `AskChannel`（内含 `UiSink`）。
   - **已完成**（`7951c03c`）：`registry.rs` 现在只导入 std 与 futures，代码中不出现任何
     crate 类型；宿主槽在 `tools/host_slots.rs`，内置工具表在 `tools/builtins.rs`。
     `registry.rs` 用 `pub use crate::tools::host_slots::*;` 保持旧路径可用。
   - `ToolCtx` 字段已全部通用，无需再迁。

**（新增，重要）抽 `agent-tool` crate 的真正阻塞点 = Rust 孤儿规则。**
跨 crate 的 inherent impl 被禁止（实测 `error[E0116]`：`impl ta::Ctx` in crate `tb` 不编译）。
若把 `ToolCtx` 移入 `agent-tool`，内核 `host_slots.rs` 里的 `impl ToolCtx { ask/install_ask/... }`
将无法编译。因此抽取前必须先决定：

- **方案 A ── 已实施（`b611f126`）**：新增 crate `agent-tool`，含 `Args`、`Extensions`、
  `ToolClass`、`ToolError`、`ToolResult`、`PendingWrite`、`WriteOp`、`is_staged_write`、
  `arg_str`/`arg_usize`/`schema_for`，以及泛型化的 `ToolSpec<C>`/`ExecFn<C>`。
  内核用别名实例化（`type ToolSpec = agent_tool::ToolSpec<ToolCtx>`），因此**零调用点改动**。
  `ToolCtx`/`ToolRegistry`/`builtins` 留内核：`dispatch` 取 `Arc<ToolCtx>`，且 inherent impl
  不能跨 crate。`cargo tree -p agent-tool` 仅列出 `agent-llm`。
- **方案 B**：`ToolCtx`/`Extensions` 也进 `agent-tool`，内核把宿主访问器改成**扩展 trait**
  （`trait ToolCtxHostExt { fn ask(&self) ... }`），调用点需 `use` 该 trait（约 30 处）。
- **方案 C**：同 B，但宿主访问器改为**自由函数**（`host_slots::ask(&ctx)`），调用点显式改形。

方案 A 不动 `ToolCtx`，风险最低；B/C 更彻底但改动面大。三者都需在新会话中单独执行与验证。
4. **T6 的残留命中**（`grep -nE 'goal|plan|blueprint|computer' engine.rs` 在 `run_turn` 内）：
   - `self.goal.reset()` / `self.plan.reset()`（回合开头）——回合生命周期，保留。
   - `submitted_plan` 取值与 plan 卡片落盘——卡片是回合交付物，引擎持有，保留。
   - `EngineTurnView` 构造、`blueprint_report` 渲染——trait 调用与呈现，保留。
   - `computer_summary`（步骤卡预览）、`turn_state.blueprint_baseline` 拍摄时机——循环数据，保留。
   判断：产品**规则**（催促、文案、上限、顺序）已全部在 `CodingPolicy`；上述是基础设施。
5. **`is_staged_write` 仍是名字集合**，不是 `ToolSpec` 元数据。原因见 `648af4c8`：
   加 `ToolSpec` 字段要改 44 处字面量，而该属性描述的是 dispatch 行为而非工具分类。已加
   `staged_write_set_matches_the_registry` 漂移测试兜底。若坚持元数据化，需一次性迁移 44 处。
6. 阶段 2–7 的剩余项见第 9 节 T7–T11。

## 5. 设计决定（不要推翻，除非有新证据）

- 能力（sandbox、desktop）经扩展表；**策略**（开关、限额、挂载）归宿主，不在工具契约里。
- “没有沙盒”（SSH 产品）→ bash **拒绝**，给出原因。“沙盒被用户关闭”→ `NoneBackend`，bash 仍运行并标明 `none`（loud-unsandboxed，保留原语义）。两者必须区分。
- 开关需**实时生效**（设置页会在运行中切换），所以不能在装配时固定后端，由 `SwitchableSandbox` 每次调用选择。
- 子代理继承沙盒时**必须同时继承限额**，否则宿主的强制网络策略会在子代理失效。
- 错误消息用 `ToolError::Failed`，不用 panic；panic 只用于构造期编程错误（如 `Engine::new` 缺 goal/plan）。

## 6. 验证方法（沿用）

- 每个阶段：全量测试 + 下游 check（含 `--tests`）+ 依赖律。
- 解耦类改动必须做**突变验证**：人为恢复旧行为（如在 `new()` 里装回探测），确认对应测试变红。突变用 Python 按函数范围替换，**替换后用 `grep -c` 或 `diff` 确认落地**，还原后再 `diff` 确认。
- 多段 `edit` 可能整体被拒绝（任一锚点不匹配则全部不生效）。每次编辑后用 `grep` 核对落地。
- 耗时验证用 `bg_run`；完成后读 `/tmp/*-summary.txt` 与各日志的 `passed/failed` 数字，**不要只看 exit code**（上一轮 exit 0 掩盖了 kernel 失败）。
- 长命令不要用 `timeout` 包整套件（会 143）；`pkill -f` 可能误杀自身 shell，改用 PID。

## 7. 建议的下一步顺序（概要，细则见第 9 节）

1. 重跑全量验证（`/tmp/sb-verify.sh` 或等价命令），确认 kernel/下游/依赖律全绿。
2. 全绿后提交沙盒迁移，提交信息写明：宿主装配点、过渡全局 `sandbox_prefs`、产品路径已验证。
3. 处理风险 2（全局状态并发）与 3（`global_skill_roots`）。
4. 迁移 `session` / `memory` / `subagent` 字段。
5. 进入阶段 2 工具包拆分。

## 8. 环境注意

- 工作区有未跟踪的 `.pi/` 运行产物（已在 `.gitignore`），`.pi/skills/rust-iced-agent-harness/` 是既有技能文件，未提交，勿误删。
- `app-desktop` 被 `Cargo.toml` 的 `exclude` 排除，不参与 workspace 构建。
- 本地 `main` 领先 `origin/main` 6 个提交，未推送。

## 9. 后续任务清单

按顺序执行。每项的“完成标准”全部满足才算完成；未满足时不进入下一项。

**进度**：T1–T6 ✅ · **T7 ✅** · **T8 ✅** · **T10 ✅** · **T11 ✅** · T9 ⬜

- T7 的接缝设计（三个 trait，全在 `agent-tool`）：`WorkspaceCtx { root, cancel_flag, resolve,
  app_paths, workspace_state_dir }`、`SandboxCtx { sandbox, sandbox_limits }`、
  `DesktopCtx { desktop }`。内核写 `impl ... for ToolCtx`（本地类型 + 外来 trait，孤儿规则允许），
  工具泛型到 `C: ...`。**拆成三个 trait 而不是一个**：文件工具不需要沙箱（SSH worker 没有），
  桌面是可选能力。
- **一个只被集成测试抓到的回归**（`e6c3ec22`）：`workspace_state_dir` 是 trait 默认方法，其体内
  `self.app_paths()` 在 `Self` 为泛型时解析到 **trait 自己的默认实现**，而非内核的固有方法——
  宿主的 `AppPathsSlot` 被静默绕过，`screenshot` 写到了 `<ws>/.agent/attachments`。
  修法是让 trait impl 覆盖 `app_paths` 并删掉固有孪生（单一实现，无法漂移）。详见该提交说明。

- T7 ✅：五个工具包全部抽出，内核不再含任何工具实现。
  `agent-tools-fs`（2631 行）、`agent-tools-web`（1137）、`agent-tools-shell`（908）、
  `agent-tools-computer`（673）、`agent-tools-office`（1169）。内核 `tools/` 从 10828 行降到 4825 行。
  内核**不再声明 `reqwest`、`htmd`、`tree-sitter`、`similar`、`strsim`、`globset`、`grep-*`**，
  文档给 T7 定的完成标准（不再直接依赖 `reqwest`/`htmd`/`tree-sitter`）已达到。
  内核残留的是编排工具（`batch`、`delegate`、`goal`/`plan` 契约）与注册表/扩展槽，这些本就是内核的。
  - **关键设计（解决了原先的阻塞）**：工具包**不能**依赖内核（会成环），而孤儿规则（E0116）又
    禁止内核给外来的 `ToolCtx` 加固有方法。于是能力以 **trait** 形式放进 `agent-tool`：
    `WorkspaceCtx { root, cancel_flag, resolve, app_paths }`，内核写
    `impl agent_tools::WorkspaceCtx for ToolCtx`（本地类型 + 外来 trait，合法），工具泛型到
    `C: WorkspaceCtx`，因此不命名任何内核类型。`ToolCtx::resolve` 已从固有方法移入该 impl。
    这是比之前列的方案 B（扩展 trait + 约 30 处 `use`）改动更小的路子。
  - 契约新增 `WorkspaceCtx::app_paths`（默认值即内核原先的 `<ws>/.agent` 兜底），`web_fetch`/
    `assets` 用它取 `app_name()` 做 User-Agent；`agent-tool` 因此依赖 `agent-paths`（零依赖叶子）。
  - `agent-kernel/src/lib.rs` 顶部「no iced, no reqwest」的依赖律注释，现在字面为真。

- T11 现状：`crates/minimal-agent` 是独立的 workspace crate，workspace 依赖只有 `agent-kernel` 与
  `agent-llm`（依赖律已钉死，突变验证过）。3 个自有工具（`ssh_exec`/`read_remote`/`list_remote`），
  脚本化 provider，无沙盒、无桌面、无 session store；一轮工具调用跑通；断言模型实际收到的 schema
  只含这 3 个工具，`bash`/`delegate`/`fuzzy_patch`/`test_runner` 均不出现。
  - **验收条件的如实记录**：首次编写时 kernel 改了一处——`Engine::run_turn` 的参数是
    `agent_context::HunkTracker`，产品被迫直接依赖 `agent-context`，于是 kernel 加了
    `pub use agent_context::HunkTracker;`（5 行，`c4617876`）。按本节规则这是“接缝切错了，回去修”。
    修好之后的一轮（补第三个工具、schema 断言，`1587ed9e`）对 kernel 与 agent-tool 的
    `git diff --stat` 为空。
  - 产品需要自己决定的一点：kernel 默认权限模式会把非只读调用挂在审批卡上，无人值守的 worker
    会永久挂起。`minimal-agent` 通过公开的 `Engine::set_permissions` 自设 `bypassPermissions`；
    测试带 10s 超时，去掉这行会以 “the turn must not park” 失败（已突变验证）。
  - 偏离文档的地方：文档写“只依赖 kernel、agent-tool、llm”。实际不需要直接依赖 `agent-tool`——
    kernel 的 `ToolSpec` 别名已覆盖；`anyhow`/`async-trait`/`tokio` 只是 dev 依赖（实现 provider 用）。
  - 文档 §10 写“T11 依赖 T7、T9”。实测在 T7 工具包拆分与 T9 未完成的情况下验收已通过：
    这两项减少的是 kernel 体积与编译依赖（第二个产品仍会编进 kernel 的内置工具代码），
    不是复用能力。是否仍要做，可以重新评估。

- T7 现状：`agent-tool`（契约）已抽出并只依赖 `agent-llm`。**工具包拆分受阻**：`bash`/`test_runner`
  依赖 `ToolCtx`、`sandbox_cfg`、`tools::util`（都在内核），若工具 crate 依赖内核而内核又注册它们
  即循环依赖。解法只有把 `ToolCtx`/`Extensions` 移入 `agent-tool`（方案 B，宿主访问器改扩展 trait，
  约 30 处加 `use`）或做依赖注入。故 T7 剩余项暂停，待决策。
- T8 ✅：`agent-skills`（零 workspace 依赖）与 `agent-store`（只依赖 `agent-llm`）均已抽出；
  内核的 `skills`/`session_store` 都是 `pub use` 转发，既有路径不变。
  - store 的公开面：`global_get/put/update`（全局行）、`SessionStore::recorded_root()`、
    `is_test_binary`、`open_or_transient`。`crate::prefs` 只用这些，不再触碰 store 内部。
  - `WorkspacePrefs` 是 store 的 per-workspace 行（`load_prefs`/`save_prefs`/迁移都读写它），
    随 store 留在新 crate；UI 侧全局偏好（`DefaultPreferences`/`AppearanceSettings`/
    `RecentWorkspace`）留在 `crate::prefs`。
- T6 完成情况：收尾分支（plan/goal/blueprint 催促）已迁入 `CodingPolicy`；computer 限额已迁入
  `before_dispatch`；分阶段写入集合已收敛为 `registry::is_staged_write` 并加漂移测试。
  `run_turn` 内剩余命中均为回合生命周期/呈现/循环数据（逐条列在第 4 节第 4 点）。
  写工具**没有**改为 `ToolSpec` 元数据，理由见第 4 节第 5 点。
- 新增测试：`goal_contract.rs`（催促上限、goal 先于 blueprint）、
  `staged_write_set_matches_the_registry`（漂移）、
  `non_desktop_calls_do_not_spend_the_desktop_budget`（配额计数范围）。
- 起步状态：`cargo test -p agent-kernel` = **349 passed / 0 failed**；依赖律与下游 check 均通过。

### T1 全量验证沙盒迁移（阻塞项）
- 操作：运行 `cargo test -p agent-kernel`，同时 `cargo check -p app-cli -p app-tui -p husk --tests`、`./scripts/check-deps.sh`。
- 完成标准：kernel 的 `passed` 与 `failed`（汇总所有 `test result` 行）为 `failed=0`；下游 check 退出码 0；依赖律输出 `dependency law: ok`。
- 注意：`bd5f05dba` 任务已不存在，不要依赖它；直接在前台或用 `bg_run` 新开。读日志里的数字，不只看退出码。
- 失败处理：若失败，先定位是迁移回归还是测试假设错误（参照第 4 节“未修复的 `ask_user`/沙盒拒绝”判断方法：拒绝类错误是 `shell execution is not available`）。

### T2 提交沙盒迁移
- 前置：T1 全绿。
- 操作：`git add crates/agent-kernel`，提交信息标题 `refactor(agent-kernel): make the process sandbox an optional host capability`。正文写明：扩展表 `SandboxSlot`/`SandboxLimitsSlot`；`SessionActor` 为宿主装配点；`SwitchableSandbox` 为过渡实现（读全局开关）；无沙盒则 bash 拒绝；`none` 后端仍运行。
- 完成标准：`git status` 对 `crates/agent-kernel` 干净；提交正文的测试数字来自 T1 的实际输出。

### T3 串行化修改全局 `sandbox_prefs` 的测试
- 文件：`crates/agent-kernel/tests/service_tool.rs` 的 `sandbox_disabled_routes_to_none`；全局为 `crates/agent-kernel/src/sandbox_prefs.rs` 的 `LIMITS`。
- 做法（二选一）：(a) 该测试文件内加一个 `static` 互斥锁，所有修改全局开关的用例先取锁；(b) 把 `SwitchableSandbox` 改为可注入的 limits 源（例如 `Arc<dyn Fn() -> SandboxLimits>`），测试注入局部值，不碰全局。推荐 (b)，因为它同时为 T4/T5 铺路。
- 完成标准：该测试在 `cargo test -p agent-kernel` 中连续跑 5 次全部通过；`grep -rn 'sandbox_prefs::set' crates/agent-kernel/tests` 无命中（若选 (b)）。

### T4 去掉 `sandbox_cfg` 对 `skills::scanner` 的直接依赖
- 文件：`crates/agent-kernel/src/tools/sandbox_cfg.rs`（`extra_ro_mounts: crate::skills::scanner::global_skill_roots()`）；测试 `sandbox_config_mounts_global_skills_read_only`。
- 做法：把 `extra_ro_mounts` 作为宿主注入的策略（可放进 `SandboxLimits` 或新的 `SandboxPolicy` 扩展槽）；`global_skill_roots()` 由宿主装配处传入。
- 完成标准：`grep -rn 'skills::' crates/agent-kernel/src/tools` 无命中；原测试改为断言注入值被原样放入 `extra_ro_mounts`；突变验证：在 `sandbox_config` 内硬编码回 `global_skill_roots()`，测试变红。

### T5 迁移 `ToolCtx` 剩余产品字段
逐个字段，每个一个提交。顺序按读取点数量从少到多：
1. `ask: Arc<AskChannel>`（读取点：`tools/ask.rs` 1 处，`engine.rs`、`session.rs` 经 `ask_channel()`）。
2. `subagent: Option<SubagentSpawner>`（`tools/delegate.rs`，构造于 `session.rs`）。`delegate` 工具本身仍在 kernel 内，若要保留通用性，把 `SubagentSpawner` 作为扩展槽、`delegate` 工具改用 `require`。
3. `memory: MemoryHandle`（`tools/remember.rs`、`tools/delegate.rs`，`session.rs` 设置）。`remember` 属于 Husk 持久记忆，应归宿主。
4. `session: Option<(i64, Arc<SessionStore>)>`（`tools/todo.rs`、`tools/delegate.rs`）。`SessionStore` 属于 `session_store` 模块，阶段 3 外移前先做扩展槽。
- 完成标准（每项）：字段从 `ToolCtx` 删除；`with_*` 安装方法保留；对应工具改用 `require`/访问器；缺失时拒绝而非 panic；`ToolCtx::new` 不默认安装；突变验证一条。
- 注意：`with_cancel` 派生 ctx 必须共享同一扩展项（`Extensions` 的 `Arc` 克隆已满足，但要有测试）。

### T6 `TurnPolicy` 接缝（阶段 4）
- 文件：`crates/agent-kernel/src/engine.rs` 的 `run_turn`（约 508–1688 行）。
- 要搬出的应用逻辑：plan 提交与 nudge（`submitted_plan`、`plan_followups`）、goal 催促（`goal_followups`、`MAX_GOAL_FOLLOWUPS`）、blueprint 返工（`blueprint_followups`、`blueprint_baseline`）、computer 动作限额（`MAX_COMPUTER_ACTIONS_PER_TURN`）、写工具暂存（`is_write` 名单，`fuzzy_patch|apply_patch|write_file`）。
- 做法：定义 `trait TurnPolicy`，钩子至少包括 `on_quiet_round`（返回 Finish 或 Nudge）、`before_tool`（Proceed / Reject / StageWrite）、`registry_for_mode`。默认实现为空，`CodingPolicy` 承接现有行为。
- 风险：blueprint 返工会重置 goal 状态（`engine.rs` 中 `self.goal.reset()` 所在分支），策略执行顺序必须显式保留。
- 完成标准：`grep -nE 'goal|plan|blueprint|computer' crates/agent-kernel/src/engine.rs` 在 `run_turn` 内无应用逻辑命中（允许出现在 trait 调用中）；`plan_mode`、`blueprint_flow`、`computer_use_*`、`office_mode`、`delegate`、`stage4`、`compaction_flow` 测试不改动且全绿。
- 注意：写工具名单与权限层 `permissions.rs` 的 `is_file_edit` 不一致（权限层多了 `fs_patch`）。先确认 `fs_patch` 是否仍需要，再合并为元数据。不要用 `ToolClass::WorkspaceMutation` 替代，因为 serena 也声明了它但不产生 `PendingWrite`，会导致审批前真实执行。

### T7 工具包拆分（阶段 2）
- 目标 crate：`agent-tools-fs`、`agent-tools-shell`、`agent-tools-web`、`agent-tools-computer`、`agent-tools-office`，外加 `agent-tool`（契约：`ToolSpec`、`ToolRegistry`、`ToolCtx`、`Extensions`）。
- 拆分顺序：先抽 `agent-tool`（需把 `registry.rs` 中对 `channels`、`session_store`、`delegate`、`engine` 的引用全部移除，依赖 T5 完成），再逐个工具包。
- 依赖注意：`assets.rs` 依赖 `net.rs`，所以 office 包依赖 web 包或共享 `net`；`todo` 依赖 `session_store`，随 store 走或留在 kernel。
- 完成标准：`cargo tree -p agent-tool` 不含任何 `agent-*` 之外的产品 crate；`scripts/check-deps.sh` 的白名单随之收紧（只删边，不加边）；kernel 的 `Cargo.toml` 不再直接依赖 `reqwest`、`htmd`、`tree-sitter`。

### T8 叶子模块外移（阶段 3）
- `session_store` → `agent-store`；先把 `DefaultPreferences`、`AppearanceSettings`、`RecentWorkspace`、`office_root` 等 Husk 偏好分出。
- `skills` → `agent-skills`。
- 这两个模块当前对 kernel 其他模块零 `crate::` 横向引用，应是机械操作。
- 完成标准：`agent-store`、`agent-skills` 不依赖 kernel；依赖律白名单更新。

### T9 宿主代码移出 kernel（阶段 5）
- 移到 `husk-host`：`session_manager.rs`（3156 行）、`sandbox_prefs.rs`、`commands.rs`、`mode.rs`，以及 `session.rs` 中的装配逻辑（含 `SwitchableSandbox` 的装配）。
- 移到 `agent-coding`：`blueprint`、`serena`、`plan`、系统提示词（`assets/kernel-system-prompt.md`）、`with_builtins`。
- 完成标准：`crates/agent-kernel/Cargo.toml` 不依赖 `agent-coding`；app-cli、app-tui、husk 编译通过；kernel 测试只保留脚本化 provider 加假工具的用例。

### T10 去除 `husk` 字面量（阶段 6）
- 范围：`agent-llm/src/config.rs`（约 8 处，`~/.config/husk`）、`agent-plugin/src/manager.rs`（约 13 处，`.husk/`、`husk/plugin-trust.json`）、kernel 约 25 处。
- 做法：引入显式传入的 `AppPaths`，由宿主构造；不用全局变量。
- 完成标准：`grep -rn husk crates/agent-llm crates/agent-plugin crates/agent-kernel crates/agent-tool --include=*.rs` 只剩注释与测试。

### T11 第二个最小 Agent 验证（阶段 7，最终验收）
- 做法：在 `examples/` 或测试 crate 中写一个 SSH 风格的最小 Agent：自定义 `LlmProvider`（脚本化）+ 3 个自定义工具（如 `ssh_exec`、`read_remote`、`list_remote`），只依赖 `agent-kernel`、`agent-tool`、`agent-llm`。不安装沙盒、不安装桌面。
- 完成标准：该 Agent 能完成一轮工具调用；`ssh_exec` 在无沙盒时仍可注册；`bash` 类内置工具不被注册时不出现在 schema 中。
- 验收条件：**此过程中 kernel 与 agent-tool 的源文件零改动**（`git diff --stat` 对这两个 crate 为空）。若需要改，说明接缝切错了，回到对应阶段修。

## 10. 任务之间的依赖

- T1 → T2：必须先全绿。
- T3 与 T4 共享“注入而非读全局”的思路，建议 T3 选 (b) 时一并完成。
- T5 → T7：`agent-tool` 抽取依赖 `ToolCtx` 不再引用产品类型。
- T6 与 T5 可并行，但 T6 会改 `engine.rs`，与 T5 中 `ask_channel()` 的接线冲突，需协调。
- T9 依赖 T5、T6。
- T11 依赖 T7、T9。

## 11. 每次工作的收尾清单

1. 更新本文件的“已提交”与“未提交”两节。
2. 任何未验证的改动必须明确写在“未验证”下。
3. 不把 exit code 当作测试结果；汇报时给出 `passed/failed` 的数字。
4. 突变验证的改动文件，还原后用 `diff` 确认与备份一致。
