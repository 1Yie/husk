+++
owns = ["crates/agent-tools-shell/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
进程工具：命令执行、测试运行器、后台服务，含沙盒配置派生与风险分级。

关键词：bash, test runner, services, sandbox config, command risk, process limits

## 职责
- `bash`（bash.rs）、`smart_test_runner`（test_runner.rs）、后台服务（services.rs）
- 沙盒配置派生（sandbox_cfg.rs）与命令风险分级（risk.rs）；office 包复用其 sandbox_cfg / util

## 不做
- 不依赖内核；工具泛型到 `SandboxCtx` / `WorkspaceCtx`
- 「没有沙盒」时拒绝并给原因；「沙盒被关闭」走 NoneBackend 仍运行并标明 none

## 对外契约
- 命令风险经 `Review.risk` 提供给权限门；沙盒限额由宿主经扩展槽注入
