+++
owns = ["crates/agent-tool/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-capabilities"]
+++

工具契约：工具声明什么、返回什么、抛什么错——不认识任何产品。

关键词：tool spec, tool class, tool result, commit, extension table, tool error, call risk, review, schema, out of scope, deny

## 职责
- `ToolSpec<C>` / `ExecFn<C>`（对上下文泛型）、`ToolClass` / `CallRisk` / `Review` / `Commit` / `ToolResult` / `ToolError`
- `Extensions` 扩展表（按 `TypeId` 键控，`insert_shared` / `inheritable`）与 `ToolContext` trait
- 参数与 schema 辅助：`arg_str` / `arg_usize` / `schema_for`
- 两种越界错误，语义不同：`PathEscape` 是**墙**（悬空符号链接、指向别处的链接——跟着它就会写到不知道哪里）；`OutOfScope { path, suggestion }` 是**问题**（这个路径不在本上下文的任何根里，但宿主也许能授予）。引擎只把后者变成授权提问，前者永远是一句拒绝
- `Review` 的三个判断：`risk`（要高于模式默认）、`action`（交给策略细分）、`deny`（这次调用无论答什么都跑不了，引擎直接记结果、不弹卡）。`deny` 的理由由工具给全，因为只有它知道为什么

## 不做
- 不命名内核的 `ToolCtx`：内核用类型别名实例化，工具泛型到 `C: ToolContext`
- 只依赖 agent-llm（工具结果可携带的线类型），不依赖任何产品
- 不决定「越界之后怎么办」：请求授权、重试还是放弃，是引擎与宿主的事

## 对外契约
- 工具包不依赖内核（避免成环）；能力经 `ToolContext` 扩展表由宿主安装
- 抛 `OutOfScope` 的工具等于声明「这条路径值得问一句」；宿主没装授权槽时，引擎把它当普通拒绝处理，工具不需要知道这件事
- 填 `Review.deny` 等于声明「这次调用注定失败」：引擎不再问人，直接把理由作为工具结果。已定输的调用不该花用户一次点击
