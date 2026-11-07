# Codex WebSocket 中途断连诊断

日期：2026-11-05。Husk 基线：`376fdadd1725d90578c37865f90d38dfea3ad58e`；
korei 依赖：`add1f430048979732f73b5511fbedb142394780e`。
用户确认症状为输出到一半断开；尚未提供关闭码、原因或事件诊断信息。
本次仅诊断，没有修改实现或访问真实服务、会话、凭据。

## 已确认：关闭原因文字改变部分输出后的重试行为

通过本机 WebSocket 服务驱动实际 `ProviderFactory → Sampler`，
两组都先发送一段正文，再发送 Close code 1000，仅改变 reason：

| reason | 请求次数 | 结果 |
| --- | ---: | --- |
| 空字符串 | 1 | fatal，保留部分输出 |
| `connection reset by gateway` | 2 | 自动重试后成功 |

两次提交的 input 相同，重试请求没有 previous_response_id。
回调累计收到 `visible-partial regenerated`。协调者独立复跑成功确认该机制。
测试断言当前异常行为，因此测试通过表示缺陷已复现。

原因链：

1. `crates/agent-llm/src/codex_transport.rs:383` 将远端 Close reason 纳入诊断文本。
2. korei `crates/agent-llm-core/src/sampler.rs:453` 用
   connection、reset、timeout 等自由文本关键词判断可重试性。
3. 同文件 `:219–240` 的重试循环没有部分输出门禁。

Transport 自身的部分输出保护因此无法阻止上层重新提交。
kernel 在重试时清空内部累积状态；此处证实的是重复生成请求，
真实 UI 是否重复显示尚未验证。

建议在重试决策边界显式保留错误类别与生成进度，避免远端 reason
决定是否重放。已开始输出的失败应明确终止并保留部分结果；
输出前允许的恢复应单独验证。改变公共错误契约与恢复策略需在修复前确认。

## 已验证与排除

现有 `cargo test --locked -p agent-llm --test codex_websocket`
退出 0，30 项通过。普通 Close、WS 400 和 malformed 事件在隔离实验中
均只提交一次；“所有 WS 错误都会触发重试”的假设被反证。

Ping/Pong 会更新活跃状态。已完成回合后的连接关闭由下一轮复用探测处理，
与生成中断的错误路径分开。连接建立、握手、复用探测与回合 idle timeout
分别存在，定义见 `codex_transport.rs` 和 korei `sampler.rs`。

## 现场原因仍未知

本次缺陷说明断连后恢复行为不一致，尚不能确定用户现场为何中途断开。
进一步对应需要脱敏后的 stage、Close code/reason、last_event、
reused/continued/recovered。真实上游、代理、网络以及服务端连接策略
均没有在本次本机模拟中验证。
