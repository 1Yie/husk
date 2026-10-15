+++
owns = ["crates/agent-tool/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-capabilities"]
+++
工具契约：工具声明什么、返回什么、抛什么错——不认识任何产品。

关键词：tool spec, tool class, tool result, commit, extension table, tool error, call risk, review, schema

## 职责
- `ToolSpec<C>` / `ExecFn<C>`（对上下文泛型）、`ToolClass` / `CallRisk` / `Review` / `Commit` / `ToolResult` / `ToolError`
- `Extensions` 扩展表（按 `TypeId` 键控，`insert_shared` / `inheritable`）与 `ToolContext` trait
- 参数与 schema 辅助：`arg_str` / `arg_usize` / `schema_for`

## 不做
- 不命名内核的 `ToolCtx`：内核用类型别名实例化，工具泛型到 `C: ToolContext`
- 只依赖 agent-llm（工具结果可携带的线类型），不依赖任何产品

## 对外契约
- 工具包不依赖内核（避免成环）；能力经 `ToolContext` 扩展表由宿主安装
