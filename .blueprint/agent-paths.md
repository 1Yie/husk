+++
owns = ["crates/agent-paths/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-store"]
+++
应用路径解析：配置/数据目录的纯路径计算，不命名任何产品。

关键词：app paths, config dir, data dir, workspace dir, path resolution

## 职责
- `AppPaths`：由宿主构造并注入，贯穿配置、插件、会话存储与工具上下文

## 不做
- 不做 IO；零 workspace 依赖
