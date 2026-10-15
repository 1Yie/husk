+++
owns = ["scripts/**", "docs/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
仓库级工具与文档：依赖律脚本与解耦接力记录，不属于任何 crate。

关键词：dependency law, check-deps, ratchet, handoff docs, workspace tooling

## 职责
- `scripts/check-deps.sh`：workspace 依赖律棘轮，新增 crate 必须在表里声明其允许的依赖
- `docs/`：阶段解耦与宿主抽取的接力记录（未跟踪，不纳入提交）

## 不做
- 不含可执行产品代码，不参与构建
