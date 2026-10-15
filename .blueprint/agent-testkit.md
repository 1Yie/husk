+++
owns = ["crates/agent-testkit/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-store"]
+++
仅测试用：跨集成测试目标共享的脚本化 LLM provider。

关键词：testkit, scripted provider, dev-only, integration tests

## 职责
- `ScriptedProvider`：按脚本回放的 `LlmProvider`，供内核与各产品的集成测试复用

## 不做
- 只经 dev-dependencies 引用；不进任何产品的正常依赖闭包
