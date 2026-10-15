# 接力：agent-kernel 解耦（阶段 2 —— T9 宿主代码外移）

> 最新实施状态见第 11 节。前面的 T9 记录保留为历史；其中“转发壳收益递减”和
> “minimal-agent 已足够”的建议已被用户要求的 Korei 纯内核边界取代。

上一轮文档：`docs/handoff-kernel-decoupling.md`（阶段 1，T1–T11 的任务表）。本文件只讲
**剩下的最后一项 T9**，并记录上一轮验证过的做法与踩过的坑。

目标不变：**产品 2 只依赖 kernel/tool/llm、零改动 kernel 即可装配**。T11 已证明这条成立；
T9 剩下的价值是**内核体积与编译依赖**（第二个产品目前仍会编进内核的编排与宿主代码）。

---

## 0. 当前状态

- HEAD `f68051c7`，本地 `main`，**36 个提交未推送**，不要 push。
- 工作区干净，只有 `docs/` 未跟踪。按项目约定**不要提交 `docs/`**（它就是接力记录）。
- 全量：**566 passed / 0 failed**（15 个 crate），下游 `app-cli` / `app-tui` / `husk` 0 error，
  `cargo check -p agent-kernel --features tree-sitter` 0 error，`./scripts/check-deps.sh` →
  `dependency law: ok`。
- `crates/agent-kernel/src/` 11867 行，其中 `tools/` 4825 行——**已不含任何工具实现**。

### 已完成，勿重做

| 任务 | 结果 |
|---|---|
| T1–T6 | 沙盒迁移、可注入开关、`ToolCtx` 产品字段全部移入扩展槽、`TurnPolicy` 接缝 |
| **T7 ✅** | 五个工具包全部抽出：`agent-tools-fs`(2631) / `-web`(1137) / `-shell`(908) / `-computer`(673) / `-office`(1169) |
| **T8 ✅** | `agent-store`（会话仓库，只依赖 `agent-llm`）、`agent-skills` |
| **T10 ✅** | 库中无 `husk` 字面量（只剩注释） |
| **T11 ✅** | `crates/minimal-agent`：只用 kernel+llm 跑通一轮工具调用 |

---

## 1. T9 要搬的东西（实测规模）

| 文件 | 行数 | 原文档指定去处 |
|---|---|---|
| `src/session_manager.rs` | 3244 | `husk-host` |
| `src/session.rs`（含装配逻辑） | 1903 | `husk-host` |
| `src/commands.rs` | 937 | `husk-host` |
| `src/mode.rs` | 137 | `husk-host` |
| `src/sandbox_prefs.rs`（只剩策略） | 112 | `husk-host` |
| `src/blueprint.rs` + `tools/blueprint.rs` | 280 + 177 | `agent-coding` |
| `src/tools/serena/`（5 文件） | 1417 | `agent-coding` |
| `src/tools/plan.rs` / `goal.rs` | 182 / 182 | `agent-coding` |
| `assets/kernel-system-prompt.md` | — | `agent-coding` |
| `tools/builtins.rs` 的 `with_builtins`/`child_builtins` | 50 | `agent-coding` |

### 关键发现：T9 不能按文件机械拆分

实测的模块互锁（`grep -rlE "crate::X" src`）：

```
commands     → 只有 session.rs 用
mode         → session.rs / session_manager.rs / engine.rs / turn_policy.rs
sandbox_prefs→ session.rs / session_manager.rs / tools/host_slots.rs / tools/registry.rs(测试)
blueprint    → commands.rs / session.rs / engine.rs / turn_policy.rs
skills       → commands.rs / session.rs / session_manager.rs / tools/skill.rs
```

`session.rs` 是这些模块的**唯一汇聚点**，而它做的是「装配」（把 provider、registry、ctx、
plugins、skills、sandbox、memory 拼成一个引擎）。所以 T9 的正切线不是「文件 → 目录」，
而是：

> **引擎（kernel）与装配（宿主）之间的那条线。**
> kernel 保留：`Engine`、`SessionActor`、`SessionConfig`、`ToolRegistry`、`host_slots`、
> `permissions`、`hooks`、`channels`、`compaction`、`turn_policy`、`agent-store` 转发。
> 宿主拿走：装配函数、`SessionManager`、斜杠命令、模式→注册表的映射、`AgentMode` 的产品含义。

---

## 2. 建议切片（顺序即依赖顺序）

### T9-1 契约面确认（无代码改动，先做，1 小时）

把「宿主需要什么」列成清单再动手，避免中途推翻：

- kernel 需要暴露 `ToolCtx` 的全部安装器（已有）、`SessionConfig`、`Engine` 的 setter 面。
- 宿主需要能构造 mode → registry 的映射。**这是 T9 里唯一需要新设计的地方**（见 §3）。

### T9-2 `agent-coding`：`serena` + `blueprint` + `plan` + 系统提示词

先搬**接缝已经存在**的那些，风险最低：

- `serena/`（5 文件 1417 行）**零外部 crate 依赖**（实测：只有 std/futures/serde_json
  + 内核的 registry）。它是和 `agent-tools-*` 完全同类的工具模块：
  `pub fn spec() -> ToolSpec` 内部 `let exec: ExecFn = Arc::new(|args, ctx| ...)`，
  只用了 `ctx.workspace_root`。**按 T7 的同一模式泛型化到 `WorkspaceCtx` 即可搬走**
  （`ExecFn` 是内核别名 `agent_tool::ExecFn<ToolCtx>`，改成泛型 `ExecFn<C>`）。
  它不依赖 `agent-plugin`——别被「MCP 桥」的名字误导，它直接 spawn 子进程。
  > 也可选择独立成 `agent-tools-serena`；文档指定去 `agent-coding`，两者都行，
  > 但若目标是让 kernel 不依赖 Coding 语义，`agent-tools-serena` 更贴合现有模式。
- `plan`（182）/ `goal`（182）工具本身是纯工具模块，但**它们的注册被 `Engine::new` 内联**，
  所以实体可以搬到 `agent-coding`，内核那一侧的接线要等 T9-3。
- `blueprint.rs`（280）被 `commands.rs`、`session.rs`、`engine.rs`、`turn_policy.rs` 引用
  （`turn_policy.rs:20` 就有 `crate::blueprint::Baseline`）。**这是本片真正难的部分**：
  `TurnPolicy` 的视图携带 blueprint 状态，搬它要么把 `Baseline` 也搬进契约，要么让
  `turn_policy` 泛型化。先做 serena/plan/goal，把 blueprint 留到 T9-3 一起。
- `assets/kernel-system-prompt.md` 跟着 `session.rs` 走；它的使用者是
  `app-tauri/backend/ipc/sessions.rs:543`（`agent_kernel::session::SYSTEM_PROMPT_TEMPLATE`）。
  搬迁后该路径要改成从 `agent-coding` 取，**这是下游唯一的破坏性改动**，先记下来。
- 完成标准：`cargo tree -p agent-coding` 只含非产品依赖；kernel 仍编译。

### T9-3 内核的注册表注入点（唯一需要新设计的一步）

**问题**：`Engine::new` 现在自己拼四个 registry：

```rust
let mut plan_reg = registry.readonly_only();
plan_reg.register(crate::tools::plan::spec_submit());
let mut goal_reg = (*registry).clone();
goal_reg.register(crate::tools::goal::spec_complete());
goal_reg.register(crate::tools::goal::spec_blocked());
// office_reg = goal_reg + office::specs() + assets::specs()
```

也就是说内核知道「plan / goal / office 模式」的概念，而这是 Coding 语义。

**候选方案**（写文档时未决，动手前先选一个并记录理由）：

- **(a) 模式表注入**：`Engine::new(..., modes: ModeTable)`，`ModeTable` 是一组
  `(AgentMode, Box<dyn Fn(&ToolRegistry) -> ToolRegistry>)`。宿主任意指定有几个模式、
  各自加什么工具。内核不再命名 plan/goal/office。
- **(b) 只留 full registry + per-call 过滤**：把「模式」降级为宿主持有的
  `Vec<(mode, ToolRegistry)>`，`set_agent_mode` 改成 `set_registry(Arc<ToolRegistry>)`。
  改动更小，但 `turn_policy` 里的 plan/goal 契约要一起重新定位。
- **(c) 不动**：接受内核内置「模式」概念，T9 只搬 `session_manager`。**这会留下 kernel
  对 Coding 语义的依赖**，与 T11 的目标不符，除非时间不够才降级至此。

`with_builtins` 的 12 个调用者（`engine.rs`、`session.rs`、`tools/{batch,builtins,delegate,registry}.rs`
及 6 个集成测试）在 (a)/(b) 下都要改。集成测试可以改成自建 registry + 显式注册。

### T9-4 `husk-host`：`session_manager` + `session.rs` 装配 + `commands` + `mode` + `sandbox_prefs`

- `session_manager.rs` 的外部引用（实测，按频次）：`session_store::{DefaultPreferences,
  is_office_root, office_root, try_load_default_preferences, save_default_preferences,
  RecentWorkspace, WorkspacePrefs, workspace_key, SessionMeta, record_recent_workspace,
  load_recent_workspaces}`、`hooks::HookChain`、`engine::PluginHandle`、
  `permissions::PermissionGate`、`mode::AgentMode`、`sandbox_prefs::{current, set,
  SandboxLimits, network_from_label}`、`channels::UiSink`、`agent_plugin::{discover,
  current_manifest}`、`skills::scanner`、`compaction::COMPACT_AT`。
  **注意 `is_office_root` / `office_root` 住在 `agent-store`**（per-workspace 目录分类），
  不是产品语义，别跟着搬。
- `sandbox_prefs` 此刻只剩 `current`/`set`/`apply_to`/`SwitchableSandbox`；
  `SandboxLimits`/`apply_limits`/`network_from_label` 已在 `agent-sandbox`（T7 时移的）。
  `SwitchableSandbox` 实现 `agent_sandbox::SandboxBackend`，宿主直接持有即可。
- 下游 `husk`(app-tauri) 的使用面：`session_manager::`×11、`session_store::`×5、
  `sandbox_prefs::`×4、`skills::`×4、`session::SYSTEM_PROMPT_TEMPLATE`×1、`mode::AgentMode`×1。
  搬完后这些 import 路径都要改，`app-cli`/`app-tui` 也要一起 grep 一遍。
- 完成标准（原文档）：kernel 的 `Cargo.toml` 不依赖 `agent-coding`；app-cli/app-tui/husk
  编译通过；kernel 测试只保留脚本化 provider + 假工具的用例。

---

## 3. 已确立的接缝模式（T7 验证过，T9 沿用）

**工具包不能依赖内核**（会成环），**孤儿规则（E0116）又禁止内核给外来的 `ToolCtx` 加固有
方法**。解法：能力以 trait 形式放进契约 crate `agent-tool`，内核写
`impl Trait for ToolCtx`（本地类型 + 外来 trait，合法），工具泛型到 `C: Trait`。

现有三个 trait（都在 `crates/agent-tool/src/lib.rs`）：

```rust
WorkspaceCtx { root, cancel_flag, resolve, app_paths, workspace_state_dir }
SandboxCtx     { sandbox, sandbox_limits }
DesktopCtx     { desktop }
```

分成三个而不是一个：文件工具不需要沙盒（SSH worker 没有），桌面是可选能力。

### 两条硬性纪律

1. **一个能力只有一个实现**。内核的 `sandbox()` / `sandbox_limits()` / `app_paths()` /
   `desktop()` / `workspace_state_dir()` 都**只有 trait impl，没有固有孪生**。
2. **trait 默认方法里不要 `self.some_accessor()`**，除非该 accessor 也在 trait 里被
   override 过。见 §4 的陷阱 1。

内核侧转发：`tools/registry.rs` 里 `pub use agent_tool::{DesktopCtx, SandboxCtx, WorkspaceCtx};`，
`tools/mod.rs` 再 `pub use registry::{...}`，所以下游可以 `agent_kernel::tools::WorkspaceCtx`。

---

## 4. 已知陷阱（都是上一轮实际踩到的）

1. **trait 默认方法解析到 trait 自己，而不是固有方法。**
   `workspace_state_dir`（trait 默认方法）体内 `self.app_paths()` 在 `Self` 为泛型时解析到
   **trait 的默认 `app_paths`**，内核那个读 `AppPathsSlot` 的固有方法被静默绕过 →
   `screenshot` 写到了 `<ws>/.agent/attachments` 而不是宿主装的 `.husk`。
   **所有单元测试通过，只有 `tests/computer_use.rs` 的端到端测试以 `NotFound` 抓到。**
   教训：跨越 trait 边界的默认方法改动，必须跑**集成**测试。

2. **抽取用 `git mv`，别用 `cp` + `rmdir`。** 上一轮 `cp` 后 `rmdir` 因目录非空静默失败，
   留下两份 `session_store/{files,migration}.rs` 死副本（其中一份已与现役版本漂移）。

3. **`pub(crate)` 会变成跨 crate 私有。** 抽取时逐个检查：`open_or_transient`、
   `is_test_binary`、`sandbox_config` 都因此放宽为 `pub`。

4. **`&Arc<C>` 不会自动 deref 成 `&C`。** 泛型化后原来靠 deref 强转的调用点要写成
   `&*ctx`，或把辅助函数签名改成 `&Arc<C>`（上一轮对 `spawn_persistent`、`search_bing`
   等用的是后者）。

5. **测试里构造的真 `ToolCtx` 不能跨 crate。** 在工具包里加一个 `#[cfg(test)] TestCtx`
   实现所需的 trait（`agent-tools-shell`、`-computer` 各有一个）。断言「内置注册表包含 X」
   这类测试属于内核（它拥有 `with_builtins`），搬到 `tools/registry.rs` 的测试模块。

6. **`Engine` 目前假定 ctx 自带 goal/plan 控制器**，`Engine::new` 里
   `.expect("engine context missing GoalController - build it with ToolCtx::new")`。
   T9-3 改装配时必须一并处理，否则第二个产品会 panic。

7. **`commands::` 在下游是假线索。** `app-tauri` 里的 `ipc::commands` 是 Tauri 的 IPC
   命令模块，和 `agent_kernel::commands` 无关。实测 `agent_kernel::commands` 全仓只有
   `session.rs` 一个使用者（做 grep 时带上 `agent_kernel::` 前缀，别只搜 `commands::`）。

---

## 5. 验证纪律（照做，否则会漏）

### 命令（并发要压住，20 核机器上全量会打满）

```sh
# 全量测试：后台跑，从日志读 passed/failed
rm -f /tmp/t.done
(nice -n 19 ionice -c3 cargo test -p <crates...> -j 4 -- --test-threads=4 > /tmp/t.log 2>&1; \
 echo "exit=$?" > /tmp/t.done) >/dev/null 2>&1 &
for i in $(seq 1 400); do [ -f /tmp/t.done ] && break; sleep 5; done
grep -E "^test result" /tmp/t.log | awk '{p+=$4; f+=$6} END {print "passed="p" failed="f}'

# 下游 + 依赖律
nice -n 19 cargo check -p app-cli -p app-tui -p husk --tests -j 4
./scripts/check-deps.sh | tail -1        # 期望 dependency law: ok
```

**不要只看 exit code**（上一轮 exit 0 掩盖过失败）。**不要用 `timeout` 包整套 cargo**
（会 143 半途杀掉）。后台任务写 `.done` 文件再轮询，注意旧的 `.done` 会造成误读——
启动前先 `rm -f`。

### 测试守恒核算

每搬一批测试，用等式核对，不要只看总数：

```
新总数 = 旧总数 − 内核移出的 + 新 crate 首次纳入测试集的
例：528 = 471 − 2(内核 sandbox_prefs 测试移走) + 59(agent-sandbox 首次纳入)
    566 = 528 + 38(agent-computer 首次纳入)
```

### 突变验证（每个解耦改动都要）

1. 破坏实现（如给新 crate 加 `agent-kernel` 依赖、去掉权限设置、改错抓取路径），
2. 确认对应测试/检查**变红**，记下实际报错文字，
3. 还原，`diff -q 备份 现文件` 必须一致。

### 提交规范

一个切片一个提交，提交信息写三节：**做了什么 + 为什么这样切**、**验证**（含 passed/failed
数字与依赖律）、**突变证据**（改了什么 → 报了什么 → 已还原）。上一轮 36 个提交都是这个格式，
可直接 `git log` 参考。

### 依赖律

`scripts/check-deps.sh` 是棘轮：新增 crate 必须在 `ALLOW` 里声明它的 workspace 依赖。
内核的白名单**只增不减地扩大过**（T7 新增了 `agent-tools-*`，因为拆分后内核仍要通过它们暴露
工具）——这一点与原文档「只删边」的表述有出入，**如实记录**，别为了对上文档而硬删边。
新叶子 crate 也要加进 `LEAF_PRODUCTS`。

---

## 6. 不要推翻的设计决定

上一轮文档 §5 的决定全部仍有效，另加：

- **能力经扩展表 / trait；策略（开关、限额、挂载）归宿主**。工具只读宿主装进 ctx 的东西。
- 「没有沙盒」（SSH 产品）→ 工具**拒绝**并给原因；「沙盒被用户关闭」→ `NoneBackend`，
  仍运行并标明 `none`。两者必须区分。
- 子代理继承沙盒时**必须同时继承限额**，否则宿主的强制网络策略在子代理失效。
- 子代理只继承**具名槽位**（`inherit_sandbox`/`inherit_desktop`/`inherit_memory`），
  绝不整体复制 `ext`——否则 goal/plan 控制器会被共享。
- 错误用 `ToolError::Failed`，不 panic；panic 只用于构造期编程错误。
- 产品需要自己决定审批策略：kernel 默认模式会把非只读调用挂在无人可答的审批卡上，
  无人值守的 worker 会永久挂起（`minimal-agent` 用公开的 `Engine::set_permissions` 自设）。
- **文档里的设计描述若与实测不符，改文档，不要改代码去迎合文档。**

---

## 7. 环境

- 20 核；**限制并发**（上面命令已带 `-j 4 -- --test-threads=4` + `nice`/`ionice`）。
- `app-desktop` 被 `Cargo.toml` 的 `exclude` 排除，不参与 workspace 构建。
- `crates/app-tauri` 的 package 名是 `husk`（`cargo check -p husk`）。
- 预存在的警告，别顺手修：`commands.rs:758` 的 `remember_memory_forget_roundtrip` 未使用。
- 未跟踪但别删：`docs/`、`.pi/`（运行产物与既有技能文件）。

---

## 8. 收尾清单

1. 更新本文件（进度、提交表、决策变更）。
2. 未验证的改动明确写在「未验证」下。
3. 汇报给 `passed/failed` 数字，不给 exit code。
4. 突变验证的文件还原后 `diff` 确认一致。

---

## 9. T9 执行记录（2026-10-14，全部完成）

| 提交 | 内容 |
|------|------|
| `6d07d6b1` | T9-2a：serena → agent-tools-serena（5 文件，泛型 over WorkspaceCtx；内核 re-export 保持调用点不变） |
| `eef0c1c1` | T9-3：方案 (a) 落地——`Engine` 改持模式表（`ModeEntry{id, registry, allow_plugins}`）+ 注入式 `TurnPolicy`；`CodingPolicy` 暂留内核但策略自持状态（Mutex 内部可变性），引擎只见泛型接缝 |
| `fd3dd922` | T9-4：宿主层 → husk-host（session_manager/session/commands/mode/sandbox_prefs + 系统提示词资产 + 14 个 SessionActor 集成测试 + probe_turn 例子；内核唯一残余 SandboxLimits 引用改指 agent_sandbox） |
| `3294cebd` | T9-2b：编码语义 → agent-coding（blueprint 层+工具、plan/goal 契约工具+控制器、CodingPolicy；内核只留 TurnPolicy trait/NoopPolicy） |

### 决策变更（与本文档正文有出入的地方）

- **T9-3 选了 (a) 的变体**：模式表不是 `(AgentMode, Fn)` 对而是 `ModeEntry{id: String, ...}`——模式 id 是纯字符串，`AgentMode` 枚举全程没进内核。`before_dispatch` 把 `architecture_enabled` 直接传给策略，`TurnView` 只在 quiet 点构建。
- **with_controllers 从内核删除**（原计划未提）：控制器类型随工具文件搬走后内核无法命名它们；`ToolCtx::new` 不再自动安装任何控制器。`agent_coding::install_contract(ctx)` 取代之（宿主装配 + 子代理策略工厂各调一次），`SubagentSpawner` 新增策略工厂参数 `Fn(&mut ToolCtx) -> Box<dyn TurnPolicy>`（默认 Noop）。
- **AgentMode 留在 husk-host**（早先草稿想搬 agent-coding）：内核已不引用它，文档 §1 本就把它归宿主。
- **测试随码走**：kernel tests 只剩 apply_patch_edges / delegate / stage3（假工具+脚本化 provider，符合完成标准）；SessionActor 装配测试 14 个 → husk-host/tests，computer_use_flow/gate（引用 CodingPolicy）→ agent-coding/tests。`tests/common` 脚本化 provider 挂架三处各存一份（kernel/husk-host/agent-coding）——git mv 纪律只适用于整体搬迁的文件，共享挂架是复制。
- **check-deps 白名单**（棘轮现状）：`[agent-coding]="agent-context agent-kernel"`；`[husk-host]="agent-coding agent-context agent-ipc agent-kernel agent-llm agent-paths agent-plugin agent-sandbox"`；husk/app-cli/app-tui 各加 husk-host；内核 ALLOW 含全部 agent-tools-*。

### 守恒核算

| 切片 | 总数 | 内核 | 变化 |
|------|------|------|------|
| 起点 | 669 | 260→248（serena 移出后） | — |
| husk-host | 669 | 153（109 lib + 44 集成） | husk-host 首纳入 95（35 lib + 60 搬入）；153+95=248 |
| agent-coding | 669 | 136（103 lib + 33 集成） | agent-coding 首纳入 17（6 lib + 11 computer-use）；136+17+95=248 |

stage3 的 `registry_lists_builtins` 断言 20→19：blueprint spec 移出 `with_builtins`，由 husk-host 装配补注册（full/readonly/child 三套 registry 都补）。

### 突变验证

- husk-host（fd3dd922）：env 门控跳过 `engine.set_policy(CodingPolicy::from_ctx)` → 5 红（plan/goal/blueprint 合同各就其位）；还原后 husk-host 95/0。
- agent-coding（3294cebd）：env 门控跳过 `install_contract` → `--no-fail-fast` 4 红（goal_contract ×2、plan_mode submit_plan ×1、blueprint_flow goal 推回 ×1）；还原后 112/0。

### 预存在 / 未修

- `agent-store` 的 `shared_db()` 进程级 OnceLock 导致 `test_model_info_with_devin_swe2` 偶发 flake（6d07d6b1 时观察一次，后续全量重跑均过）——下轮从源头修。
- husk-host 两条 lib-test 警告原样搬迁：未使用的 `use crate::mode::AgentMode`（commit 1 起 dead）、`remember_memory_forget_roundtrip` 未使用。未顺手修。

### 未验证

- 真实 UI 端到端（app-tauri 装配进 Tauri 后的交互流）未跑——三slice均以 workspace 测试 + check --all-targets 验证。
- 36+ 提交未推送（本纪律不变），push 仍留给用户。

---

## 10. 接手状态（2026-10-14，第二轮收尾）

### 已完成

- T9 四个切片（`6d07d6b1` `eef0c1c1` `fd3dd922` `3294cebd`）。
- **flake 修复**（`ccea971f`）：根因**不是** `shared_db()` 的 OnceLock，而是 husk-host 测试：
  1. `test_paths_with_config()` 用 `fs::write`（先截断再写）改共享 `config_dir/config.toml`，并发的
     `AppConfig::load` 读到空文件 → provider 变 "no provider configured"。改为临时文件 + `rename`。
  2. `reload_model_config_...` 靠 config.toml 的 mtime 判断"已变化"，共享目录被兄弟测试在同一 mtime
     刻度内改写 → 加载被跳过。改用私有 config 目录。
  压测（16 线程并发整套 lib 测试）：基线 600 次失败 19 次，修复后 1000 次 0 失败。`shared_db()` 无需改动。
- **agent-testkit**（`a504979b`）：四份 `ScriptedProvider` 副本（kernel/husk-host/agent-coding 三份相同 +
  app-cli/app-tui 经 `#[path]` 借用）合并为 dev-only crate，依赖律里为叶子，只许 dev-dependencies 引用。
- **kernel 依赖瘦身**：`agent-tools-office`、`agent-tools-serena` 不再是 kernel 的正常依赖。
  office 本就只有 husk-host 在用，直接改依赖；serena 从 `with_builtins` 移到 husk-host 装配注册
  （full 与 child 两套 registry），kernel 内 3 个测试改为自行补注册。`with_builtins` 现为 18 个。
- **T11 复跑**：minimal-agent 仍只依赖 kernel + llm，`cargo tree` 中 husk-host / agent-coding /
  office / serena 反向边 0 条，测试 2/2 绿。
- workspace 670/0（新增 base_registries 同步测试）；`scripts/check-deps.sh` ok。

### 审计修复（`f500a6cc` `d9fb5bf8`）

- **模式 id 常量**：`agent_coding::MODE_PLAN` / `MODE_GOAL`，策略、宿主模式表、`AgentMode` 共用，改名不再静默失效。
- **`architecture_enabled` 移出引擎**：改为 `CodingPolicy` 持有的 `ArchitectureSwitch`（共享原子开关），宿主保留句柄；
  `TurnView` 与 `before_dispatch` 签名随之瘦身。
- **`Verdict` 通用化**：`Finish{before_final, after_final: Vec<Effect>}`，`Effect{history, ui}`；引擎只按序落盘，
  不再认识 plan 卡片 / blueprint 报告。plan 卡先于最终消息、报告后于最终消息，两个顺序已被测试钉住，
  突变（互换 / 合并）均能红。
- **装配约束**：`host_tools()` + `base_registries()` 让父/子 registry 共用同一份宿主工具；
  `CodingPolicy::has_contract()` + 宿主 `debug_assert`，防止 `from_ctx` 早于 `install_contract`。
- agent-coding 依赖律新增 `agent-ipc agent-llm`（Effect 携带 UiEvent / ChatMessage）。

### 审计后续（`f8bedb86` 及之后）

- **prefs 整体外移**：`DefaultPreferences` / `AppearanceSettings` / 最近工作区读写整个模块从 kernel 搬到
  `husk_host::prefs`（kernel 内无人使用，只是被 `session_store` 转发）。kernel 因此不再持有任何用户设置，
  也不再引用 `is_office_root`；app-tauri 调用点改指 `husk_host::prefs`。未拆结构——整体搬迁后混合字段
  是宿主自己的事，拆分收益不再成立。
- `delegate.rs` 中写死的 `.husk/…` 文案已改成 `<workspace_dir>/…`。

### 审计遗留（有意未动）

1. 进程级全局：`agent-store::shared_db`、`husk-host::sandbox_prefs::LIMITS`、`agent-tools-shell::SERVICES`、
   `agent-tools-serena::MANAGERS`。`LIMITS` 是设置页 / CLI 旗标"一次改全部会话"的共享真相源（23 处调用），
   一个进程只有一个设置源；注入口已存在（`SwitchableSandbox::with_limits`），代码注释已写明。
   只有当两个产品共进程时才需要改。
2. kernel 的 `session_store` / `skills` 只是 `pub use` 转发壳；去掉要改动大量 import，收益有限。

### 环境约束

- 本机 cargo 默认拉满核心与内存会 OOM：一律 `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 nice -n 10 cargo … -j4`。

### 未完成

- 推送：本地 main 领先 origin/main，按纪律未 push。
- 真实 UI 端到端（Tauri 交互流）未跑。
- husk-host 两条旧警告（`use crate::mode::AgentMode`、`remember_memory_forget_roundtrip`）与
  `cargo fmt --check` 的存量差异，按纪律未顺手修。

### 后续

- `session_store`（56 处内部引用）与 `skills` 是否随宿主走：T11 已复跑通过，证据支持"不阻塞第二产品"，
  收益递减，可按体积需求再定。
- 若动 `SubagentSpawner` 的策略工厂：默认 Noop 工厂不装控制器，子代理 goal/plan 工具拒绝是设计行为。

## 11. Korei 纯内核继续拆分（2026-10-14）

用户明确要求：独立内核项目叫 **Korei**，Husk 和未来的 AI SSH 产品依赖它。
验收标准是只组装通用 agent 引擎即可工作；文件、shell、工作区、sandbox、桌面、memory、
skills、session store、blueprint 等能力由产品自行选择。
用户已明确说明 `rust-iced-agent-harness` 技能过时，不再作为本轮设计依据。

### 已提交且不要重做

- `044d7823`：写入延后为通用 `agent_tool::Commit { verify, apply }`，引擎不再理解文件写入；
  `ToolSpec.staged` 决定是否预执行，写跟踪由 agent-coding 的 observer 提供。
- `b5749bda` / `6161caba`：`Extensions::insert_shared` / `inheritable()`；
  子代理只继承宿主标记共享的槽，不再逐项调用 `inherit_*`。
- `4a47c7c0`：remember 与 MemoryHandle 归 agent-coding；kernel 不再依赖 agent-context。
- `8a209e9d`：skill 工具归 agent-coding；删除 kernel 的 skills 转发壳和 agent-skills 依赖。
- `467a02f1`：todo 使用宿主注入的 `TodoStorage`，Husk 提供 session 数据库实现；
  旧 `.agent/todos.json` 先成功持久化再删除，坏文件保留；SessionSlot 只持有 id。
- `8b135603`：删除 kernel 的 session_store 转发壳和 agent-store 依赖；
  会话数据库诊断 examples 搬到 husk-host。
- `43244804`：`ToolSpec.review` 提供摘要、动作和审批前异步准备；引擎不再按 `computer` 名称分支。
- `0350fedf`：通用 `ToolPolicy`；桌面审批、launch 独立授权归
  `agent_coding::computer_policy::ComputerPolicy`，由 Husk 显式安装。
  通用 gate 保留已有 Ask/Deny，不能被策略放宽；headless / dontAsk 不允许策略新增无人回答的 Ask。
  该步 workspace **685 passed / 0 failed**，依赖律通过。
- `98f2d417`：kernel 默认工具只有 delegate / ask_question / todo / batch_execute，
  没有任何 `agent-tools-*` 直接依赖或重导出。Husk 的 `session::base_registries()`
  装配 20 个工具并过滤子代理的 delegate / remember / computer / HumanInteraction。
  文件、补丁、子代理、桌面验收测试共 44 个随能力搬到 husk-host，均保留。
  tree-sitter 转发改归 husk-host。该步 workspace **688 passed / 0 failed**，依赖律通过。
- `d7796715`：`Review.risk: CallRisk` 由工具提供；shell 分类归 agent-tools-shell/risk.rs。
  kernel 不再解析 command 字符串或按名字识别文件工具；未知工具名和参数不会自行获得权限。
  保留原有命令分类和规则优先级，新增 bash / test-runner 元数据矩阵与真实引擎审批验证。
  该步 workspace **693 passed / 0 failed**，依赖律通过。
- `f12f4e8b`：通用 `SubagentHost { resolve, instructions, persist }`；匿名委托默认不读 manifest、
  不加宿主指令、不存文件。345 行目录/manifest/指令/transcript 实现与四个单元测试归
  husk-host/subagents.rs，Husk 和 Tauri 入口已切换；kernel 不再依赖 dirs。
  新增无宿主适配器的真实子引擎测试。该步 workspace **694 passed / 0 failed**，依赖律通过。
- `fc4edbb8`：通用 `ToolContext`（Extensions 读写、取消、typed service）；
  app paths 槽和方法归可选 `agent-capabilities::WorkspacePaths`。
  kernel / agent-tool 不再直接依赖 agent-paths；截图、web、Husk 已迁移。
  该步 workspace **694 passed / 0 failed**，依赖律通过。
- `5c304caa`：sandbox / desktop 契约、槽和 builder 归 agent-capabilities 的独立 features，
  默认 features 为空；kernel / agent-tool 无后端直接依赖或类型引用，五个缺省/继承测试归宿主。
  `agent-capabilities --no-default-features` 编译通过，workspace **694 passed / 0 failed**，
  依赖律通过。注意 plugin 的传递依赖尚未切断。
- `06cc7ee2`：裸 `ToolCtx::new()`；WorkspaceCtx / root / resolve 与原路径算法归
  agent-capabilities/workspace.rs，三个路径边界测试归宿主，新增缺失 root 拒绝与 root 共享测试。
  CodingPolicy 显式接收宿主路径。all-targets check 通过，workspace **696 passed / 0 failed**。

### 当前未提交步骤

- `a2c2e2e3`：kernel `plugins::{PluginRouter, PluginHandle}`，未归属返回 None（替代字符串哨兵）；
  command-hook 适配归 husk-host/plugins.rs；Husk 持 RuntimePluginHandle，引擎用 router_handle 适配器，
  热重载可见；wasm feature 归 husk-host。kernel 正常闭包 = agent-ipc / agent-llm / agent-tool。
  workspace **696 passed / 0 failed**。

用户决定（2026-10-14）：产品事件改为 `UiEvent::Custom { kind, payload }` 并同步改 Tauri 前端与 TUI；
`NoticeKind::Plan` 泛化为 UI-only 卡片，旧 `"plan"` 行必须继续可读。

- `c3fceeff`：`NoticeKind::Card(String)`，手写 serde：保留名映射原变体，其余字符串为卡片 kind，
  旧 `"plan"` 行原样可读；`is_ui_only()` 统一 Compacted / Card。全量 695 passed / 2 failed，两例均为
  agent-sandbox host_socket_isolation（该 crate 无工作区依赖，本改动不影响其测试二进制；
  隔离重跑 3 次中 1 次失败，bwrap 在只读 ~/.config 下 mkdir 失败，已知环境 flake）。

- `4daef62c`：`UiEvent::{PlanSubmitted, BlueprintReport}` → `UiEvent::Custom { kind, payload }`
  （`plan_submitted` / `blueprint_report`），同步 agent-coding、app-tui、两宿主测试与 Tauri 前端
  （`CustomEvent` 判别联合）。cargo check + 前端 tsc 通过；全量 696 passed / 1 failed，失败为
  service_tool `persistent_service_lifecycle`（固定 800ms 等待与 0.6s 脚本竞争，未触及本次代码，隔离 5/5 通过）。

- `03ab303b`：`UiCommand` 原样移到 `husk-host/src/command.rs`；内核 `UiChannels<C>` / `ui_channels::<C>()`
  对命令类型泛型，内核与 agent-ipc 不再命名 UiCommand；22 个消费者改指 `husk_host::command::UiCommand`；
  JSON 形状不变，前端无改动。全量 697 passed / 0 failed。

`92e152f9` batch 切片（全量 700 passed / 0 failed）：内核 `batch_execute` 不再写死 coding 工具名。
- 新增 `agent_kernel::tools::batch::BatchProfile { description, example_call, infer }`；
  `spec()` = `spec_with(BatchProfile::default())`，用通用描述、通用形状提示，且不按签名推断工具名。
- `BatchCall.tool` 的文档注释（进入模型 schema）去掉了工具名。
- Husk 在 `base_registries()` 里用 `spec_with(batch_profile())` 覆盖注册；profile 带原描述
  （字符串值与旧版逐字相同，994 字节）、原示例与原推断表（url→web_fetch，pattern/regex/max_hits→smart_grep，
  command→bash）。注意 `command` 键同时是既有的名称别名，与推断无关。
- 测试：内核新增 `the_generic_profile_never_guesses_a_tool_or_names_one`；宿主 `tool_registry.rs`
  新增描述不变与无名条目推断两项。

审计结论：
- 三处模型可见产品措辞已在 `ffabd8dd` 泛化（见下）。
- `engine.rs` 审批预览按 path/command/pattern/query/file/action/url/target 取定位参数，是通用启发式，
  工具可用 `Review.summary` 覆盖，保留。

`ac8713d0` minimal-agent 去留（用户决定：删 crate，测试移入内核；全量 700 passed / 0 failed）：
- 其依赖约束与 check-deps 中 `[agent-kernel]` 行重复；但它的端到端回合测试别处没有。
- `crates/minimal-agent` 已 `git rm`；两项测试原样移到 `crates/agent-kernel/tests/standalone_assembly.rs`
  （集成测试按外部 crate 编译，只能用公开 API；无需新增 dev 依赖）；check-deps 删除对应行与 LEAF 名。
- AI SSH 只是用户举例，不做专门演练。

Korei 闭包审计（agent-llm / agent-ipc / agent-tool / agent-kernel）：
- `dd6f3852` DevConfig 切片（全量 700 passed / 0 failed）：`agent_llm::config::DevConfig`（renderer、gpu_acceleration、
  devtools、developer_ui 等 GUI 启动设置）原样移到 `app-tauri/backend/dev_config.rs`，仅 Tauri 使用；
  agent-llm 删除未使用的 `dirs` 依赖。
- 其余命中均可保留：`<attached-image path=…/>` 是引擎的图片输入约定（通用）；`ToolError` 的
  "path escapes workspace" 是工具契约中的通用错误；`CompactionPlan` 中的 plan 与产品 plan 无关。
- `ffabd8dd` 措辞切片（用户同意；全量 700 passed / 0 failed）：`compaction.rs` 改为
  "[image dropped: superseded by a newer image]"（同步测试断言）、`rig_bridge.rs` 改为
  "(images from the tool results above)"、`ask.rs` 改为 "inspecting the environment yourself"。
- agent-llm 仍带 keyring / toml_edit / rig-core / reqwest：提供方配置与凭据解析是 LLM 层合理职责，暂留。

### 本轮产品边界（已完成）

以下三项已分别验证并提交；最新全量结果为 703 passed / 0 failed：

1. `d1d69cd0` blueprint 归编码产品层（编译、依赖检查通过；全量 700 passed / 0 failed）：
   - `agent-context/src/blueprint/{mod,imports,manifest}.rs` 迁到 `agent-coding/src/blueprint/`；
     原编码层回合接入移到同目录的 `runtime.rs`，现有 `agent_coding::blueprint` 入口保留。
   - 示例 `blueprint_check` 同迁，报告类型和工具导入改指编码层。
   - `globset`、`toml` 仅供 blueprint 使用，依赖随模块从 agent-context 迁到 agent-coding。
   - 首次检查 `bec3c310e` 因两处 `MAX_INDEX_ENTRIES` 仍引用 `crate::workspace` 失败，
     尚未开始测试；已改指 `agent_context::workspace`。逐字比对迁移前后，解析器、
     检查规则与回合逻辑均未改变，只有模块路径及公开接入调整。
2. `a04ba1c7` `office_root` / `is_office_root` 归 husk-host（编译、依赖检查通过；全量 701 passed / 0 failed）：
   - 路径和判断移到 `husk-host/src/office.rs`；CLI/TUI/SessionManager 统一调用
     `office::init_store`，Tauri 的办公路径入口同步迁移。
   - `agent-store::workspace_key` 删除办公目录特判，新增通用
     `init_data_dir_with_namespaces`：宿主为数据目录下的根注入稳定名称。
     Husk 注入 `("office", "office")`，保留旧键，普通目录继续用 canonical-root hash。
   - 新增 `office_storage` 集成测试：保存、重开、last_active、符号链接路径，
     以及名为 office 的普通项目仍与办公会话隔离。
3. `5434f3b6` sandbox/computer 品牌清理（编译、依赖检查通过；全量 703 passed / 0 failed）：
   - computer 线程名改为 `agent-libei`、libei 客户端名改为 `agent`，macOS 提示为
     “restart the app”；示例、测试和截图路径说明同步去品牌。无持久化迁移要求。
   - sandbox 默认屏蔽集合及审计计划不再写死 `.config/husk` / `.config/agent-rs`；
     宿主 `sandbox_prefs::protected_paths` 提供实际 `AppPaths.config_dir` 加历史目录。
   - shell/测试/服务经 `SwitchableSandbox::with_deny_read` 注入；普通命令和
     `wrap_spawn` 都合入 `SandboxConfig::deny_read`，保留调用方已有的屏蔽路径。
   - plugin `load_all`、`hooks_from_manifest`、`McpClient::start` 接收通用屏蔽列表；
     管理器默认保护其 `AppPaths.config_dir`，Husk 加入历史路径；MCP probe 同步注入。
   - 新增 host 短命令/持久进程路径传递测试，以及插件 hook 工厂的保护路径测试。
   - 静态复核：agent-sandbox / agent-computer / agent-tools-computer / agent-capabilities
     的 Husk/agent-rs 品牌命中均为零；agent-store 无 office 产品逻辑；
     工作区 normal/build 闭包仍为 agent-kernel / agent-llm / agent-tool / agent-ipc。

其余状态：插件、事件/命令、卡片边界已提交；`extract_attached_images` 已审计为通用图片输入约定。
AI SSH 是用户举例，不再安排专门演练；独立组装检查由内核集成测试守护。
当前三个边界切片全部完成，代码工作区无待提交改动；docs 保持未跟踪，没有推送。
独立 Korei 仓库准备仍是后续工作。

### 本轮纪律

- 不推送；docs 保持未跟踪，不纳入提交。
- 一次一个职责切片，独立提交，每步全量后台测试。
- Cargo 构建/测试一律 `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 nice -n 10 cargo … -j4`。
- 后台 cargo 命令保留原始退出码和完整日志。不要用 `grep | awk` 的成功退出掩盖 cargo 失败；
  完成后再用 context-mode 汇总日志。
- `agent-sandbox` 的 host_socket_isolation、`agent-plugin` 的执行脚本 ETXTBSY
  有历史环境偶发失败；当前两次已提交步骤全量均为零失败，不要把未见失败提前归为 flake。
- 不顺手修 husk-host 两条旧警告或存量格式差异。
