+++
owns = ["crates/agent-plugin/**"]
forbidden_deps = ["agent-kernel"]
invariants = ["manifest 未声明的权限一律拒绝"]
+++
插件系统：manifest、MCP stdio 桥、WASM 运行时、hook 声明。

关键词：plugin manifest, mcp, json-rpc, wasm, hook command, capability declaration

## 职责
- 插件加载、能力声明校验、MCP 客户端
- 单个 hook 命令的拉起与 JSON 判决解析（`hooks.rs`）；WASM 运行时在 `wasm` feature 之后

## 不做
- 不编排 hook 链与回合生命周期（属于 kernel）

## 对外契约
- hook 线协议：stdin 一份 JSON 载荷，stdout 一份 JSON 判决；失败一律降级为 continue
