+++
owns = ["crates/agent-context/**"]
forbidden_deps = ["agent-kernel", "agent-llm", "reqwest"]
invariants = ["召回块按增量生成：已在 `seen` 里的事实不得再次进入 facts 块", "persona 的 workspace 分区不跨工作区泄漏；`all_persona` 只合并本分区与全局行", "`set_persona_scoped` 返回的 key 必须是真正写入的那一行——折叠不得只改返回值", "召回必须校验路径型事实的真实性：路径全不存在的事实不得进入提示块", "写入去重与 persona 折叠只看哈希向量 `embedding`；语义向量只参与召回打分与「词不共而意同」的块内去重", "语义后端不可用（未开 feature、模型未就绪、加载失败）时行为与只有哈希向量时一致", "语义推理不得在 redb 写事务内进行，也不得占用异步 worker", "提示块预算与每行上限按字符计，不按字节", "判断 persona 某 key 是否已存在只看同一作用域；同名跨作用域是两条独立的行，会话记账必须用 `delta_key`", "`last_used_at == 0` 的旧行不得被按年龄淘汰：那一列在本版之前不存在", "模型文件只有通过大小与 sha256 校验后才能落到正式文件名并进入加载；校验失败的文件删除，不得留给下一次", "ONNX Runtime 的 Environment 进程内只建一次，重试复用它", "加载线程的任何失败（含 panic）都必须落到 `Failed` 并带上完整错误链，不得停在下载中", "失败态不得被 `set_enabled` 自动重试：一次会话启动只尝试一次，重试只能由 `retry()` 发起", "旧缓存复用失败只能降级，不得让整次拉取失败"]
+++

工作区感知层：文件树扫描、git 快照、写入追踪、分层记忆。

关键词：workspace scanner, file index, git snapshot, hunk tracker, undo, memory, redb, embeddings, semantic embeddings, fastembed, model download, turn distiller, persona scope, recall delta, persona merge, fact lifecycle, purge, model stamp, char budget

## 职责
- `WorkspaceScanner` / `WorkspaceTree`（workspace.rs）、git 快照（git.rs）
- `HunkTracker`：按回合记录写入、外部改动标记与撤销计划（hunks.rs）
- 分层记忆存储（memory/，redb + 向量检索）与回合蒸馏接缝（`TurnDistiller`）
- 两种向量并存：`Fact.embedding` 是按词哈希向量，确定、零依赖，负责写入去重（0.92）、persona 折叠，以及召回块内「说的是同一句话」那一半去重。`Fact.semantic` 是可选的语义向量（`semantic` feature，fastembed 多语言模型），负责召回打分，外加块内去重的另一半：只有「一个词都不共」却读起来相同的两条，才算同一条事实的两种语言。`Fact.semantic_model` 记录产出这条向量的模型——两个不同的模型可以对出同样的维度，长度因此不能当兼容性判据；对不上模型的向量视为缺失并重算
- 语义后端（memory/semantic.rs，仅 `semantic` feature）：进程级单例，模型在后台线程懒加载，加载完成前一律走哈希路径；就绪后缺向量的事实由 `embed_batch` 按批补算（每批 `SEMANTIC_BACKFILL_BATCH`）并回写。`ENABLED` 是设置页那道开关：关闭时 `embed` 直接返回 `None`，模型留在内存里不参与任何召回；`available()` 报的是构建是否带模型（设置页据此决定画不画开关），`is_ready()` 才是「此刻真的在语义打分」。失败不会被下一次会话启动自动重试——每个会话都会调 `set_enabled`，不挡住就变成每开一个会话重下一次 240 MB
- 模型获取（memory/semantic_fetch.rs）：模型文件由本模块自己下载到宿主给的缓存目录（`<cache>/<repo 名>/<文件>`），不经 hf-hub、不看 `HF_HOME`。两个出口依次尝试：环境变量或系统代理（macOS 读 `scutil --proxy`），再直连——代理配了却没跑时连接会被立即拒绝，没有直连这一档就没有答案。TLS 用系统证书库校验（公司根证书、抓包工具的根证书都认）；每个请求有连接与读超时、有限次重试、按 `Range` 续传；落盘前按清单里的大小与 sha256 校验，通过才改名为正式文件。端点默认 Hugging Face，`HF_ENDPOINT` 可改，主端点连不上时退到镜像。旧版 hf-hub 缓存里校验通过的文件直接复制过来（快照项是指向 `blobs/` 的符号链接，只能复制内容，硬链接会带走相对目标）；这一步是尽力而为，失败只降级不中断。加载用 `try_new_from_user_defined`，ONNX Environment 进程内只建一次
- 下载进度是真实字节计数：已校验文件的大小 + 正在下载文件已写入的字节，总量取自文件清单；同时报当前文件名与完整错误链（含试过的出口）
- persona 两个作用域：`PersonaScope::Global` 跟随用户跨工作区，`Workspace` 只在本工作区分区可见；`/forget` 两个作用域一起清。同名不同作用域是两条独立的行，会话记账用 `Persona::delta_key()`（`<scope>:<key>`）区分，否则值不同的那条会随每条消息重复下发
- persona 写入折叠：新 key 的值与**同一作用域**已有条目足够接近时并入那条，写入返回落点 key——否则列表每个会话长一个近义词。判断「这个 key 是否已存在」只看同作用域，跨作用域会把写入推进另一个分区
- fact 生命周期：`last_used_at` 记录最后一次被注入模型的时间，排序与退休都看它；`evict_stale_facts` 按「多久没被用到 + confidence」退休，人工写入的（`FactSource::Manual`）不因年龄淘汰，而 `last_used_at == 0` 的旧行（`FactSource::Legacy`）跳过——那一列在本版之前不存在，0 不是「从未用过」的证据，按它退休会一次性删掉整库
- fact 真实性校验：文本里带路径的事实，在召回时按工作区根校验存在性——路径全都不存在就跳过（文件搬了、crate 改名了，这条事实说的已经不是这棵树）。绝对路径与 `~/` 开头的不参与（它们不以工作区根为基准）
- 召回打分：相关度绝对下限 + 相对最佳命中的比例，两种向量各有一套下限（`RELEVANCE_FLOOR` / `SEMANTIC_FLOOR`）；confidence 与「最近被用到」只影响排序，不相干的尾部不再随块下发。ANN 索引答不满 `k` 时补一次精确扫描——索引落后于存储时，少一条事实比多扫一遍更糟
- 召回块内去重：与块里已选事实过于相近（`FACT_TWIN_SIMILARITY`）的事实不占名额，但照样记为已送达
- 预算按字符计：`FACTS_BLOCK_BUDGET` / `STABLE_BLOCK_BUDGET` 与每行上限都是字符数——中文一个字三个字节，按字节算会让 640 的额度只够一条半事实，四个名额形同虚设
- 蒸馏只从“这一轮真的发生了什么”取证据：回合回答（`TurnRecord.reply`）参与模型归纳，纠偏/拒绝只留在 episode 的 `correction` 里；模型那一遍能看到已在用的 persona key，才谈得上「复用已有 key」；事实带 `durable` 分级（stated / observed / guess → 0.8 / 0.6 / 0.4），只说「X 在哪个目录」的事实一律按 guess
- 旧数据迁移：`purge_polluted` 一次清掉旧规则写下的 steer/denial 事实、重复回合，把目录类事实降到 `LAYOUT_FACT_CONFIDENCE`；每个工作区分区在每个 `PURGE_VERSION` 下第一次被打开时自动跑一次（META 里的 `purged:v<N>:<ws>` 标记）

## 不做
- 不调用 LLM；蒸馏所需的模型调用由宿主（husk-host）注入。本地嵌入模型不是 LLM 调用
- 不含架构蓝图——blueprint 已归 agent-coding
- 不决定“注入什么”：会话内去重、历史回滚与压缩前修剪由宿主按历史维护
- 默认构建不含语义模型：`semantic` 是可选 feature，下载与加载失败只降级，不报错
- 不持久化这个开关：开关状态属于用户偏好，由宿主经 `set_enabled` 传下来
- 不用 reqwest：`semantic` 在 fastembed 之外只直接依赖 `ureq`（下载，带平台证书校验）、`sha2`（校验）与 `ort`（进程级 Environment，版本钉死与 fastembed 一致）。HTTP 客户端不与 LLM 层共享，也不引入 fastembed 的 hf-hub
- 不在 redb 写事务里做语义推理，也不把整表召回跑在异步 worker 上：写事务是全库唯一的写锁，推理是秒级 CPU；两者都在调用方（宿主）用阻塞线程池隔开

## 对外契约
- `HunkTracker`：按回合记录写入，供撤销与宿主使用
- `MemoryStore` / `TurnDistiller`：记忆读写与回合蒸馏的接缝
- `facts_block_excluding(prompt, seen)`：按消息召回的增量块，返回已用 id 供宿主记账，并把选中事实的 `last_used_at` 推进；`facts_block` 是它空 `seen` 的版本
- `stable_block()` 只含 persona——episodes 不再进系统提示，改由 `search_report` 按需读；`stable_block_carried()` 额外返回真正塞进预算的条目
- `persona_delta_block(seen)`：会话中途写入的偏好，作为增量随下一条消息下发；`seen` 以 `Persona::delta_key()` 为键
- `set_persona_scoped(scope, key, value) -> key`：返回真正落点的 key（可能被折叠到已有条目）；`set_persona` 是全局作用域的 `()` 版本；`persona_in(scope, key)` 只看该作用域，`persona(key)` 是本工作区优先、全局兜底
- `search_report(query, limit)`：`memory_search` 工具的读路径（相关事实 + 最近回合）
- `evict_stale_facts(max_age_days, min_confidence)`：生命周期退休，只处理本构建记录过用量的行；`purge_polluted() -> PurgeReport` 做旧库迁移，打开时自动跑一次，`/memory purge` 可再手动触发
- `semantic::set_enabled(on, cache_dir)` / `available()` / `is_ready()` / `embed()` / `embed_batch()` / `model_id()`：宿主启动时按偏好调一次；未开 feature 时同一组函数是空实现（`embed` 恒 `None`、`model_id` 恒空串、`available` 恒 `false`）
- `semantic::progress() -> Progress { phase, bytes, total, file, error }`：只读内存里的计数，不扫目录，可高频轮询；`error` 是完整错误链，含试过的出口
- `semantic::retry()`：失败后重新下载/加载（复用已校验的文件，且是唯一会重试失败态的入口）；`semantic::cancel()`：中止进行中的下载，阶段回到 `Idle`，已下完并校验的文件保留。`set_enabled(false)` 同样会中止下载
- `Fact` 新增的 `last_used_at` / `source` / `semantic` / `semantic_model` 都带 serde 默认值，旧行照常可读——所以 `SCHEMA_VERSION` 仍是 2，只有真正改了存储形态才需要升
