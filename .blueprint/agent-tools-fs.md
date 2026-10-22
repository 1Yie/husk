+++
owns = ["crates/agent-tools-fs/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++

工作区文件工具：读取、补丁、列目录、搜索、结构大纲。

关键词：fs tools, read, apply_patch, fuzzy_patch, list_dir, grep, outline, tree-sitter, linked roots

## 职责
- `read` / `apply_patch` / `fuzzy_patch` / `list_dir` / `smart_grep`（fs_read / apply_patch / fs_patch / list_dir / grep）
- 写入以 `Commit` 延后提交（commit.rs）；`outline` 在 tree-sitter feature 之后
- 读工具（read / list_dir / smart_grep）走 `resolve_read`，可达主根与全部关联根；写工具走 `resolve`，只达主根与读写关联根。`smart_grep` 在关联根内搜索时，glob 匹配相对该关联根的路径，结果行给绝对路径——相对路径永远指主根

## 不做
- 不依赖内核：工具泛型到 `WorkspaceCtx`，只读宿主装进 ctx 的工作区
- 不决定关联哪些目录，也不询问用户：越界就报 `PathEscape`，是否借此发起关联由宿主决定

## 对外契约
- 写工具经 `Commit { verify, apply }` 交引擎在权限决定之后落盘
