+++
owns = ["crates/agent-sandbox/**"]
forbidden_deps = ["agent-kernel", "agent-llm"]
invariants = ["环境变量走白名单，*_KEY/*_TOKEN/*_SECRET 永不透传", "强制 deny_write（.git 内部、rc 文件、IDE 任务目录）覆盖每一个可写根，不只主工作区"]
+++

L2 进程沙箱：bwrap/landlock、环境净化、资源上限、命令审计。

关键词：sandbox, bwrap, landlock, seatbelt, shell audit, network policy, proxy, env sanitize, process limits, extra roots

## 职责
- `SandboxBackend::run_command` 与平台后端
- shell AST 审计与网络策略代理
- 多根审计：`audit_command_rooted(cmd, workspace, extra_roots)` 把落在额外根内的路径视为在范围内；`audit_command_scoped` 是无额外根的版本
- `SandboxLimits.extra_rw_mounts`：宿主授予的额外读写根，`apply_limits` 并入 `extra_rw_mounts`，且对每个额外读写根同样计算强制 `deny_write` 集合

## 不做
- 不决定命令是否被允许执行（权限门属于 kernel）；只审计并标注风险等级
- 不知道会话与模型

## 对外契约
- `SandboxBackend` trait（`run_command` / `tier`）；后端自行完成环境净化、资源上限与超时杀进程树
