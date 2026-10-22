+++
owns = ["crates/agent-tools-shell/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++

进程工具：命令执行、测试运行器、后台服务与后台任务，含沙盒配置派生与风险分级。

关键词：bash, test runner, services, background jobs, bg_run, sandbox config, command risk, process limits, linked roots

## 职责
- `bash`（bash.rs）、`smart_test_runner`（test_runner.rs）、后台服务（services.rs）与后台任务 `bg_run`（bg.rs）
- 日志读写（log.rs）由 service / bg_run 共用；写入走私有目录与 0600 文件，不跟随符号链接
- 沙盒配置派生（sandbox_cfg.rs）与命令风险分级（risk.rs）；office 包复用其 sandbox_cfg / util
- `sandbox_cfg::prepare(ctx, command, timeout)` 是进程工具唯一的审计入口：把主根、关联根的读写划分交给 `audit_command_rooted`，再把关联根按访问级别并入 `extra_ro_mounts` / `extra_rw_mounts`。四个调用方（bash / test_runner / bg_run / office）都走它，所以「一次测试运行」不会比「一次 shell 运行」少一层边界
- `risk::classify_in` 比 `classify` 多知道一件事：**有没有东西真的在约束这条命令**。沙盒关着（`none` 后端或没装后端）时，写到主根与所有读写关联根之外的命令升级为 `RequiresApproval`——那时没有任何机制阻止它，问一句比默默执行好

## 不做
- 不依赖内核；工具泛型到 `SandboxCtx` / `WorkspaceCtx`
- `bg_run` 只认宿主注入的 completion notifier，不认会话 / UI / 内核类型；通知文本里的日志尾部一律标注为数据
- 「没有沙盒」时拒绝并给原因；「沙盒被关闭」走 NoneBackend 仍运行并标明 none
- 不决定哪个目录可关联：风险分级只看上下文里已经装好的 `LinkedRoots`

## 对外契约
- 命令风险经 `Review.risk` 提供给权限门，按 action 分级：list/status/logs/wait 只读，run 走命令分级；沙盒限额由宿主经扩展槽注入
- 后台任务按 owner 注册（`BgRegistrySlot`，子代理共享，供父会话查看与停止）；完成通知的 sink 按上下文安装（`install_notifier`，不共享），子代理的任务不会向父回合上报
- 任务结束即释放 pid，注册表只对存活任务发信号
