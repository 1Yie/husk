# 右侧子代理状态与计数诊断

日期：2026-11-06。Husk 基线：`376fdadd1725d90578c37865f90d38dfea3ad58e`。
用户报告子代理运行中状态或数量错误。本次只诊断，未修改实现。

## P2：从历史恢复的等待卡片在 resumed 后被误判完成

合法场景：

1. 子代理提问，delegate 返回等待文本，结果写入历史。
2. 重新加载聊天视图，由 `viewFromHistory` 恢复等待卡片。
3. 主代理调用 `answer_child`，内核发出 `subagent:resumed`，子代理继续运行。

状态探针使用真实 `viewFromHistory`、`applyEvents`、`sessionChildren` 和摘要计算：

| 时点 | 期望状态 | 实际状态 | 活跃数 | 总数 |
| --- | --- | --- | ---: | ---: |
| 等待卡片恢复后 | waiting | waiting | 1 | 1 |
| 收到 resumed 后 | running | done | 0 | 1 |

胶囊显示“1 个子代理已完成”，右侧卡片 DOM 显示“已完成”，并把旧等待文本当报告。
总数保持 1 符合列表契约；错误发生在运行状态、活跃计数及摘要。
协调者独立运行状态断言，实际退出码 126，断言预期 running、实际 done。

原因链：

- `crates/app-tauri/frontend/src/features/chat/hooks/view-from-history.ts:343`
  从 delegate 的等待返回恢复卡片，同时保留 `content` 并设置 `waiting`。
- `crates/app-tauri/frontend/src/features/chat/hooks/apply-event.ts:1175`
  处理 resumed 时只移除 `waiting`，旧 `content` 仍在。
- `crates/app-tauri/frontend/src/features/chat/hooks/running-children.ts:118`
  先检查 waiting，再以 content 是否存在区分 running 与终态，因此恢复运行后被读成 done。
- korei `crates/agent-kernel/src/tools/delegate.rs:2300` 的 `answer_child_exec`
  在交付回复后发送 resumed，并继续等待下一结果；该事件表示恢复执行。

建议在生命周期事件的归一化边界明确恢复运行状态，清除等待返回留下的终态字段，
并保持侧边栏、胶囊与活跃计数使用同一状态投影。回归应覆盖历史恢复后的
waiting → resumed → running → done，而不只覆盖纯实时卡片。

## 已排除与验证范围

- 纯实时卡片的 waiting → resumed 正常。
- 后台子代理跨父回合结束仍保持运行，未复现提前完成。
- `agentRowsByChild` 遍历完整 items；跨用户回合工具记录丢失的初始候选被反证。
- 定向现有测试通过；上述历史恢复后的 resumed 边界由新增隔离探针复现。

浏览器探针使用实际组件和合成合法事件，采集到等待与完成状态 DOM，
没有 pageErrors。截图的侧栏位于视口外，视觉截图验收未完成。
本次没有运行真实子代理，也没有读取用户会话。

尚未确定用户现场是否恰好经过聊天视图重载或历史恢复；其他运行中状态异常
需要对应事件或截图进一步区分。本次没有确认详情点击错配或后台任务状态缺陷。
