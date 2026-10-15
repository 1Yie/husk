+++
owns = ["crates/agent-tools-office/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
文档工具：officecli 驱动的 Office 文档读写。

关键词：office, docx, xlsx, pptx, officecli, document tools

## 职责
- Office 文档工具（office.rs），复用 agent-tools-shell 的沙盒配置与进程工具 util

## 不做
- 不依赖内核；办公模式的产品语义归宿主

## 对外契约
- 与进程工具同受宿主注入的挂载/网络策略约束
