+++
owns = ["crates/agent-sandbox/**"]
forbidden_deps = ["agent-kernel", "agent-llm"]
invariants = ["环境变量走白名单，*_KEY/*_TOKEN/*_SECRET 永不透传"]
+++
L2 进程沙箱：bwrap/landlock、环境净化、资源上限、命令审计。

关键词：sandbox, bwrap, landlock, seatbelt, shell audit, network policy, proxy, env sanitize, process limits

## 职责
- `SandboxBackend::run_command` 与平台后端
- shell AST 审计与网络策略代理

## 不做
- 不决定命令是否被允许执行（权限门属于 kernel）；只审计并标注风险等级
- 不知道会话与模型

## 对外契约
- `SandboxBackend` trait（`run_command` / `tier`）；后端自行完成环境净化、资源上限与超时杀进程树
