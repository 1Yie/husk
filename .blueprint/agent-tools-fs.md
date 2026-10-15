+++
owns = ["crates/agent-tools-fs/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
工作区文件工具：读取、补丁、列目录、搜索、结构大纲。

关键词：fs tools, read, apply_patch, fuzzy_patch, list_dir, grep, outline, tree-sitter

## 职责
- `read` / `apply_patch` / `fuzzy_patch` / `list_dir` / `smart_grep`（fs_read / apply_patch / fs_patch / list_dir / grep）
- 写入以 `Commit` 延后提交（commit.rs）；`outline` 在 tree-sitter feature 之后

## 不做
- 不依赖内核：工具泛型到 `WorkspaceCtx`，只读宿主装进 ctx 的工作区

## 对外契约
- 写工具经 `Commit { verify, apply }` 交引擎在权限决定之后落盘
