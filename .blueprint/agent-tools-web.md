+++
owns = ["crates/agent-tools-web/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
invariants = ["网络只经 web_fetch / assets 等显式工具（net.rs）"]
+++
网络工具：网页抓取与资源获取。

关键词：web fetch, assets, http, markdown extraction, net

## 职责
- `web_fetch`（web_fetch.rs）与资源获取（assets.rs），共用 `net.rs`
- User-Agent 取宿主注入的 `app_paths().app_name()`

## 不做
- 不依赖内核；工具泛型到 `WorkspaceCtx`

## 对外契约
- 跨网络边界的工具在 `ToolSpec.network` 上标注，受更紧的批处理上限约束
