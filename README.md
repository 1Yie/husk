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

## 简介

Husk 是一个在本机运行的编码 Agent。它能读代码、改代码、跑测试、查资料。写操作需要你确认，命令在沙箱里执行。

编排逻辑都在一个和界面无关的 Rust 内核里，桌面端和 CLI 是接在内核上的两个适配层。


## 特性

### 三种模式，共用一套工具注册表

| 模式 | 可用工具 | 回合何时结束 |
| --- | --- | --- |
| 构建 build | 完整工具集，可读写、可执行 | 模型不再调用工具 |
| 计划 plan | 只读工具加 `submit_plan` | 同上 |
| 目标 goal | 完整工具集加 `goal_complete` / `goal_blocked` | 模型声明目标达成或受阻 |

计划模式会生成结构化的方案卡，包含步骤、涉及文件、验证方式和风险。

### 内置工具

文件与检索类工具有 `smart_read`、`list_dir`、`smart_grep`、`fuzzy_patch`、`apply_patch`、`bash`、`smart_test_runner`、`web_fetch`。

编排类工具有 `batch_execute`（批量执行观察类调用，一次最多 16 个）、`delegate`（子代理）、`todo`、`ask_question`、`skill`，以及 `serena`。`serena` 接入了 [Serena](https://github.com/oraios/serena)，提供 `find_symbol`、`replace_symbol_body`、`rename_symbol` 等符号级操作。

### 桌面操作（computer use）

`screenshot` 用于只读观察，抓取屏幕并把 PNG 交给模型。`computer` 用于驱动鼠标键盘，支持点击、拖拽、输入、按键、滚动和等待。

- 坐标：`screenshot` 会打印图片分辨率和真实分辨率。模型按图片坐标给 `computer` 传参，后端再换算成真实屏幕坐标。
- 图片回传：截图会随工具消息进入下一轮请求。Anthropic 的 `tool_result` 自带 image 块。Chat Completions、Responses、Gemini 三种协议的 tool 输出只接受文本，所以截图改为放进紧随其后的一条合成 user 消息里。模型没有声明支持 `image` 输入时，只返回文件路径。
- 权限：`default` 和 `acceptEdits` 下每次动作都要确认，`dontAsk` 直接拒绝，`auto` 和 `bypassPermissions` 自动放行。`plan` 模式只保留 `screenshot`，能看不能操作。子代理的注册表里没有 `computer`。
- 后端：目前支持 X11，依赖 `xdotool` 和 `scrot`（或 ImageMagick 的 `import`）。PNG 缩放到长边不超过 1568，由纯 Rust 的 `image` 库完成。没有 `$DISPLAY` 或缺少依赖时，工具会返回明确的安装提示。
- 沙箱：桌面后端不走 `bwrap`。`--clearenv` 和 `--tmpfs /tmp` 会让 X socket 无法访问，工具在沙箱里根本用不了。这一块靠权限门把关，没有文件系统隔离。

### 子代理

`delegate` 会启动一个上下文干净的新 Agent。它和主 Agent 共享工作区，但没有 `delegate` 工具。

- 单个任务默认可写，加上 `readonly: true` 后只读。
- 同时派发 2 到 3 个并行任务时，一律只读。
- 预算为 48 轮工具调用或 5 分钟。
- 自定义子代理写成 `<name>.md`，放在 `.husk/agents/` 或 `~/.config/husk/agents/`，也兼容 `.agents/agents/` 和 `.claude/agents/`。内置了 `review`、`test`、`ui-design` 三个。

### 模型接入

支持 `openai_compat`、`openai_responses`、`anthropic`、`gemini` 四种适配器。可以并列配置多个 provider，并用 `fallback_chain` 设置降级顺序。

- 密钥可以写成 `env:VAR` 或 `keyring:<service>/<account>`。
- 请求体和日志会做脱敏。
- 内置 SSE 空闲超时、指数退避重试、doom loop 检测和流中断续写。

### 上下文与持久化

- 上下文用到约 80% 时自动压缩，前面的内容会被压成一条 `NOTE`。压缩检查点设在每个回合开始时，以及每两个工具轮之间。界面会显示压缩卡片，也可以用 `/compact` 手动触发。
- 会话按工作区保存为 JSONL 文件，侧栏可以置顶会话、查看历史。
- 粘贴到输入框的图片会存到 `<workspace>/.husk/attachments/`。

### 权限与沙箱

权限分五档，从宽到严依次为 `bypassPermissions`、`auto`、`acceptEdits`、`default`、`dontAsk`。规则优先级为 `deny > ask > allow`。

`bash` 和测试运行器都通过 `SandboxBackend::run_command` 执行，这一层做了以下限制。

- 环境变量走白名单，`*_KEY`、`*_TOKEN`、`*_SECRET` 一律拦截。
- 设有资源上限，超时后会清理整个进程树。
- 命令会解析成 shell AST 并分级审计，分为关键、提权、网络变更、越界四类。

### 扩展：插件、MCP、Hook、技能

插件就是一个 `manifest.json`，可以提供 `tools`、`context_providers`、`commands`、`hooks` 四类能力。

- MCP 使用 stdio 上的 JSON-RPC 2.0 客户端，支持 `tools/list_changed` 热重载。
- Hook 是本地命令，有 `on_user_input`、`before_tool_execute`、`after_tool_execute`、`on_state_transition` 四个事件，`sdks/` 里有 Python 和 TypeScript 的 SDK。
- 技能是 `SKILL.md` 文件，放在 `.agents/skills/`，也兼容 `.claude/skills/` 和 `.pi/skills/`。在对话中用 `/name` 或 `$name` 调用。

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

- Rust stable 和 Cargo
- Node.js 20+ 和 npm
- Tauri CLI 2，用 `cargo install tauri-cli --version "^2"` 安装
- Linux 系统库

  ```bash
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev libdbus-1-dev libssl-dev patchelf rpm
  ```

- 可选安装 `bwrap`（bubblewrap）

### 开发运行

```bash
npm ci --prefix crates/app-tauri/frontend   # 前端依赖
cd crates/app-tauri
cargo tauri dev                             # 启动桌面端（Vite 起在 1420）
```

使用独立的 app 标识：

```bash
cargo tauri dev -c tauri.dev.conf.json5
```

只跑前端（Tauri API 用 stub）：

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

配置文件是 `~/.config/husk/config.toml`，优先读 TOML，也接受 `config.json`。

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

下面是一个拦截 `git push --force` 的插件。

`~/.config/husk/plugins/git-guard/manifest.json`

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

`guard.py`

```python
from husk_hooks import on, run, veto   # sdks/python/husk_hooks.py

@on("before_tool_execute")
def guard(p):
    if "push --force" in p["args"].get("command", ""):
        return veto("force-push needs a human")

run()
```

工作区级的插件和 MCP 放在 `.husk/plugins` 与 `.husk/mcp`，用户级的放在 `~/.config/husk/plugins` 与 `~/.config/husk/mcp`。信任记录保存在 `~/.config/husk/plugin-trust.json`。

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
| `agent-ipc` | `UiCommand` / `UiEvent` 的定义与编解码 |
| `agent-kernel` | 工具注册表、ReAct 循环、权限门、压缩、会话、技能、子代理 |
| `agent-llm` | Provider 适配器、SSE 采样、重试与降级、密钥解析 |
| `agent-context` | 文件树扫描、git 快照、hunk 追踪 |
| `agent-sandbox` | bwrap 后端、命令审计、环境脱敏、资源限制 |
| `agent-computer` | 桌面控制后端，包括抓屏（纯 Rust 缩放）和合成输入，支持 X11 与 `none` |
| `agent-plugin` | 插件清单、MCP stdio 桥、hook 链 |
| `app-tauri`（`husk`） | Tauri 后端，以及 React / Tailwind / Radix 前端 |
| `app-cli`（`agent-cli`） | headless 事件泵 |

## 许可证

[MIT](LICENSE)