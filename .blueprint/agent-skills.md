+++
owns = ["crates/agent-skills/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-store"]
+++
技能：按需加载的可复用指令集（`SKILL.md`）——是知识，不是能力。

关键词：skills, SKILL.md, skill catalog, skill scanner, skill loader, frontmatter, did-you-mean

## 职责
- 扫描有哪些技能（只读 frontmatter）、按需读取单个技能正文、生成目录块与调用提示框架
- 兼容 `.agents/skills/`、`.claude/skills/`、`.pi/skills/`

## 不做
- 不执行技能内容，不触碰会话状态；skill 工具与调用时机由 agent-coding 决定
- 不依赖内核或任何产品
