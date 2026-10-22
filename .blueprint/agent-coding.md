+++
owns = ["crates/agent-coding/**"]
forbidden_deps = ["husk-host", "app-cli", "app-tui"]
invariants = ["内核不得反向依赖本 crate（agent-coding → agent-kernel 单向）", "goal / plan 控制器每上下文一套，绝不跨子代理共享"]
+++

编码产品语义：plan / goal / blueprint 回合契约、blueprint 读写工具与写守卫、观察路由、编码回合策略。

关键词：plan mode, goal mode, blueprint, blueprint_edit, write guard, architecture check, coding policy, computer policy, remember, memory_search, skill tool, write tracker, observation routing, observation ledger, repeat read, grep context, linked workspaces

## 职责
- 回合契约与控制器：`goal` / `plan`（goal.rs / plan.rs），由 `install_contract` 安装
- `CodingPolicy` / `coding_policy_factory`（policy.rs）：催促上限、blueprint 返工、computer 动作限额、`ArchitectureSwitch`
- blueprint 解析/依赖图/检查（blueprint/mod.rs）：加载与渲染共用 `parse_node_file` / `render_node_file`，写出的节点必然是能读回的节点
- 节点唯一写入口 `blueprint_edit`（blueprint/edit.rs）：字段化修改、写入前校验、staged 提交，`reason` 随结果回显
- 写守卫 `guard_patch`（blueprint/guard.rs）：目标落在 `.blueprint/` 的调用即拒绝并指向 `blueprint_edit`，无论路径写在顶层（`path` / patch 头）还是元工具的 `arguments` 里
- 回合末基线（blueprint/runtime.rs）：`.blueprint/` 是隐藏目录、工作区遍历看不见，因此节点文件单独纳入快照；绕过工具改动的节点被点名
- 只读工具 `blueprint`（blueprint_tool.rs）：locate / read / check / graph
- 观察路由的运行时一半（observe.rs）：`smart_read` / `smart_grep` / `list_dir` / `bash` 各包一层，只在结果后追加注记——`ObservationLedger`（每上下文一份，batch 派生上下文共享）认出同参同结果的重复观察、同参同错的重复失败、`content_hash` 已变的文件；纯读工作区文件的 `bash` 命令在结果末尾附路由提示。工具本身的职责描述与 `smart_grep` 的 `context` 属于能力层（`agent-tools-fs` / `agent-tools-shell`），不在这里重复
- 「纯读」的判定必须真的没有写路径（observe/shell.rs）：`sed` 光有 `-n` 不算读——`s///w`、`s///e`、`1e cmd`、`w file` 都在写或执行，`-f`（脚本来自文件）与 `-i` 一样放弃分类；`find` 的 `-fprint*`/`-fls` 同类。误判的代价是给一条错提示，所以规则一律向「不分类」倾斜
- 越界提示指向许可而不是绕路：`PathEscape` 的注记告诉模型请用户关联该项目（一次关联覆盖整个目录，只读或读写），`bash` 只解一次燃眉之急——否则模型会用 shell 绕过整套边界
- 桌面审批 `ComputerPolicy`（computer_policy.rs）、写入追踪 `WriteTracker`（tracker.rs）
- 记忆工具（remember.rs）：写 `remember`（默认本工作区作用域，`global: true` 才跨工作区）、读 `memory_search`（只读，事实 + 最近回合）。两者都在阻塞池上执行——读写都要开 store 的唯一写事务，开了语义模型时还要先跑推理
- 其余编码侧工具：`skill`（skill.rs）

## 不做
- 不装配会话（属于 husk-host）；不实现文件/shell 等能力（属于 agent-tools-*）
- 不依赖 husk-host 或任何前端
- 不重写工具描述、不补工具参数：那是能力层的事，宿主侧重复一份就会两处漂移
- 观察路由不拦截、不缓存替代真实读取：每次调用都执行内层工具，结果原样返回，注记只追加
- 不知道自己被授予了哪些关联工作区：路径可达性由能力层的 `LinkedRoots` 槽决定，本 crate 只负责把失败说清楚

## 对外契约
- `MODE_PLAN` / `MODE_GOAL` 与宿主模式表共用；策略自持状态，经 `TurnPolicy` 接缝交给引擎
- `blueprint_tool::spec()` / `blueprint::edit::spec()` / `blueprint::guard_patch`：宿主注册读工具与写工具，并用守卫包住自己的 patch 工具
- `observe::{read, grep, list, bash}` 只包装执行、不碰 schema；`ObservationLedger::install` 由宿主装在根上下文、由 `coding_policy_factory` 装在子代理上下文；`observe::shell_observation` 供统计工具复用同一分类
- `remember::spec()` / `remember::search_spec()`：宿主注册记忆的写入口与读入口；两者共用一个可切换的 `MemoryHandle`，记忆关闭时统一拒绝
