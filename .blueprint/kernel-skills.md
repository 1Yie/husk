+++
owns = ["crates/agent-kernel/src/skills/**"]
forbidden_deps = ["kernel-core", "kernel-tools", "kernel-store"]
+++
技能：按需加载的可复用指令集（`SKILL.md`）。

关键词：skills, SKILL.md, skill catalog, skill scanner, skill loader, slash invocation

## 职责
- 扫描有哪些技能（只读 frontmatter）、按需读取单个技能正文、生成目录块与调用提示框架
- 兼容 `.agents/skills/`、`.claude/skills/`、`.pi/skills/`

## 不做
- 不执行技能内容，不触碰会话状态；调用由 kernel-core 与 skill 工具负责
