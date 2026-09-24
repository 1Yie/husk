<div align="center">

<img src="crates/app-tauri/icons/icon.png" alt="Husk" width="112" />

# Husk

![version](https://img.shields.io/github/v/tag/1Yie/husk?style=flat-square&label=version&color=2f6feb&sort=semver)
![rust](https://img.shields.io/badge/rust-stable-dea584?style=flat-square&logo=rust&logoColor=white)
![tauri](https://img.shields.io/badge/tauri-2-24c8db?style=flat-square&logo=tauri&logoColor=white)
![platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-6e7681?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-3fb950?style=flat-square)

</div>

---

## Husk 是什么

Husk 是一个跑在本机的编码 Agent，读代码、改代码、跑测试、查资料，写操作经过权限确认，命令在沙箱里执行。

编排逻辑在一个与界面无关的 Rust 内核里，桌面端和 CLI 是接在内核上的两个适配层。


## 特性

### 三种模式，一套工具注册表

| 模式 | 可用工具 | 回合何时结束 |
| --- | --- | --- |
| **构建 build** | 完整工具集，可读写、可执行 | 模型不再调用工具 |
| **计划 plan** | 只读工具 + `submit_plan` | 同上 |
| **目标 goal** | 完整工具集 + `goal_complete` / `goal_blocked` | 模型显式声明目标达成或受阻 |

计划模式产出结构化方案卡（步骤、涉及文件、验证方式、风险）。

### 内置工具

文件与检索：`smart_read`、`list_dir`、`smart_grep`、`fuzzy_patch`、`apply_patch`、`bash`、`smart_test_runner`、`web_fetch`。

编排：`batch_execute`（批量观察调用，最多 16 个）、`delegate`（子代理）、`todo`、`ask_question`、`skill`、`serena`（[Serena](https://github.com/oraios/serena) 桥接，`find_symbol`、`replace_symbol_body`、`rename_symbol` 等符号级操作）。

### 子代理

`delegate` 派生干净上下文的新 Agent，共享工作区，无 `delegate` 工具。

- 单任务默认可写；`readonly: true` 限只读。
- 2–3 个并行任务强制 `readonly`。
- 预算：48 轮工具调用 / 5 分钟。
- 自定义写成 `<name>.md`：`.husk/agents/`、`~/.config/husk/agents/`（兼容 `.agents/agents/`、`.claude/agents/`）。内置 `review` / `test` / `ui-design`。

### 模型接入

适配器：`openai_compat`、`openai_responses`、`anthropic`、`gemini`。多 provider 并列配置，`fallback_chain` 降级链。

- 密钥支持 `env:VAR`、`keyring:<service>/<account>`。
- 请求体与日志脱敏。
- SSE 空闲超时、指数退避重试、doom loop 检测、流中断续写。

### 上下文与持久化

- 上下文用到约 80% 自动压缩，前缀压成 `NOTE`。压缩检查点在每回合开始与每个工具轮之间。界面显示压缩卡片，`/compact` 手动触发。
- 会话按工作区落盘为 JSONL，侧栏支持置顶与历史查看。
- 图片粘贴进输入框，存到 `<workspace>/.husk/attachments/`。

### 权限与沙箱

五档权限模式：`default`、`acceptEdits`、`auto`、`dontAsk`、`bypassPermissions`。规则优先级 `deny > ask > allow`。

`bash` 与测试运行器经 `SandboxBackend::run_command`：

- 环境变量白名单、`*_KEY` / `*_TOKEN` / `*_SECRET` 黑名单、资源上限、超时清理进程树。
- shell AST 解析与审计分级（关键 / 提权 / 网络变更 / 越界）。

### 扩展：插件、MCP、Hook、技能

插件即 `manifest.json`，四类能力：`tools`、`context_providers`、`commands`、`hooks`。

- **MCP**：stdio JSON-RPC 2.0 客户端，支持 `tools/list_changed` 热重载。
- **Hook**：本地命令，四个事件（`on_user_input`、`before_tool_execute`、`after_tool_execute`、`on_state_transition`），Python / TypeScript SDK（`sdks/`）。
- **技能**：`SKILL.md`，`.agents/skills/`（兼容 `.claude/skills/`、`.pi/skills/`），`/name` 或 `$name` 内联。

## 架构

```
┌──────────────────────── 前端 ─────────────────────────┐
│        husk (Tauri 2 + React)   │    agent-cli        │
└────────────────┬────────────────┴──────────┬──────────┘
                 │        agent-ipc          │
                 │   UiCommand  /  UiEvent   │
┌────────────────▼───────────────────────────▼──────────┐
│                     agent-kernel                      │
│  ReAct 循环 · 工具注册表 · 权限门 · 压缩 · 会话 · 技能   │
└───┬──────────────┬──────────────┬──────────────┬──────┘
    │              │              │              │
agent-llm    agent-sandbox   agent-context   agent-plugin
 provider 适配器  bwrap / 审计   工作区 / git      MCP / hooks
```

## 快速开始

### 环境要求

- Rust stable 与 Cargo
- Node.js 20+ 与 npm
- Tauri CLI 2：`cargo install tauri-cli --version "^2"`
- Linux 系统库：

  ```bash
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev libdbus-1-dev libssl-dev patchelf rpm
  ```

- 可选：`bwrap`（bubblewrap）。

### 开发运行

```bash
npm ci --prefix crates/app-tauri/frontend   # 前端依赖
cd crates/app-tauri
cargo tauri dev                             # 启动桌面端（Vite 起在 1420）
```

独立 app 标识：

```bash
cargo tauri dev -c tauri.dev.conf.json5
```

仅前端（Tauri API stub）：

```bash
cd crates/app-tauri/frontend && npx vite --config vite.harness.config.ts
```

### 打包

```bash
cd crates/app-tauri
cargo tauri build     # deb / rpm / nsis / msi / dmg
```

### headless CLI

```bash
cargo run -p app-cli -- --headless --workspace . --prompt "解释这个仓库的结构"
```

### 测试

```bash
cargo test
```

## 配置

`~/.config/husk/config.toml`（TOML 优先，也接受 `config.json`）。

```toml
active_provider = "deepseek"
active_model    = "deepseek-chat"
# fallback_chain = ["backup"]

[providers.deepseek]
kind     = "openai_compat"            # openai_compat | openai_responses | anthropic | gemini
base_url = "https://api.deepseek.com"
api_key  = "env:DEEPSEEK_API_KEY"     # 或 keyring:<service>/<account>

[[providers.deepseek.models]]
id             = "deepseek-chat"
name           = "DeepSeek Chat"
reasoning      = true
input          = ["text"]             # ["text", "image"] 表示多模态
context_window = 128000

[providers.deepseek.models.cost]
input  = 0.14
output = 0.28

[providers.deepseek.models.thinking_level_map]
low  = "low"
high = "high"
```

## 插件示例

`~/.config/husk/plugins/git-guard/manifest.json`：

```jsonc
{
  "id": "git-guard",
  "name": "Git Guard",
  "version": "1.0.0",
  "capabilities": {
    "hooks": [{
      "event": "before_tool_execute",
      "filter": { "tool": "bash" },
      "run": { "command": "python3", "args": ["guard.py"] }
    }]
  }
}
```

`guard.py`：

```python
from husk_hooks import on, run, veto   # sdks/python/husk_hooks.py

@on("before_tool_execute")
def guard(p):
    if "push --force" in p["args"].get("command", ""):
        return veto("force-push needs a human")

run()
```

插件目录：`.husk/plugins`、`.husk/mcp`（工作区），`~/.config/husk/plugins`、`~/.config/husk/mcp`（用户级）。信任记录：`~/.config/husk/plugin-trust.json`。

## 数据目录

| 路径 | 内容 |
| --- | --- |
| `~/.config/husk/config.toml` | Provider 与全局偏好 |
| `~/.config/husk/AGENTS.md` | 全局指令 |
| `~/.config/husk/{plugins,mcp,agents}` | 用户级插件、MCP 服务器、子代理 |
| `~/.local/share/husk/sessions/<ws_hash>/` | 会话 JSONL、`index.json` 索引、会话级状态 |
| `<workspace>/.husk/` | 附件、工作区插件与 MCP、子代理 |

## 项目结构

| Crate | 职责 |
| --- | --- |
| `agent-ipc` | `UiCommand` / `UiEvent` 契约与编解码 |
| `agent-kernel` | 工具注册表、ReAct 循环、权限门、压缩、会话、技能、子代理 |
| `agent-llm` | Provider 适配器、SSE 采样、重试与降级、密钥解析 |
| `agent-context` | 文件树扫描、git 快照、hunk 追踪 |
| `agent-sandbox` | bwrap 后端、命令审计、环境脱敏、资源限制 |
| `agent-plugin` | 插件清单、MCP stdio 桥、hook 链 |
| `app-tauri`（`husk`） | Tauri 后端 + React / Tailwind / Radix 前端 |
| `app-cli`（`agent-cli`） | headless 事件泵 |

## 许可证

[MIT](LICENSE)
