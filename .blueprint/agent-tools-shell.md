+++
owns = ["crates/agent-tools-shell/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++

进程工具：命令执行、测试运行器、后台服务与后台任务，含沙盒配置派生与风险分级。

关键词：bash, test runner, services, background jobs, bg_run, sandbox config, command risk, process limits

## 职责
- `bash`（bash.rs）、`smart_test_runner`（test_runner.rs）、后台服务（services.rs）与后台任务 `bg_run`（bg.rs）
- 日志读写（log.rs）由 service / bg_run 共用；写入走私有目录与 0600 文件，不跟随符号链接
- 沙盒配置派生（sandbox_cfg.rs）与命令风险分级（risk.rs）；office 包复用其 sandbox_cfg / util

## 不做
- 不依赖内核；工具泛型到 `SandboxCtx` / `WorkspaceCtx`
- `bg_run` 只认宿主注入的 completion notifier，不认会话 / UI / 内核类型；通知文本里的日志尾部一律标注为数据
- 「没有沙盒」时拒绝并给原因；「沙盒被关闭」走 NoneBackend 仍运行并标明 none

## 对外契约
- 命令风险经 `Review.risk` 提供给权限门，按 action 分级：list/status/logs/wait 只读，run 走命令分级；沙盒限额由宿主经扩展槽注入
- 后台任务按 owner 注册（`BgRegistrySlot`，子代理共享，供父会话查看与停止）；完成通知的 sink 按上下文安装（`install_notifier`，不共享），子代理的任务不会向父回合上报
- 任务结束即释放 pid，注册表只对存活任务发信号
