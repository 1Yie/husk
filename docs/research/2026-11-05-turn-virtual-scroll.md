# 主聊天 turn 虚拟滚动诊断

日期：2026-11-05。源码基线：`94c915114d484134a7765063111363dd90a94d9e`。
范围：内容不可达、滚动跳动及卡顿分析；未修改实现。

## P1：从底部直接上滚到顶后，较早正文不可达

Chromium 156 与 WebKit 27.2 均以真实 `mouse.wheel` 输入复现。
场景包含 72 个普通正文 turn、144 个 items，`historyStart=0`，
全部历史已经加载；初始渲染窗口保留最新 30 个 turn。

一次大幅向上滚动到顶，等待 700ms，再继续向上滚动三次，
最早可见 turn 仍是 turn42。全部 144 个 items 仍在 store。
向下滚动 700px 再向上滚动 2000px 后，恢复 8 个 turn，窗口起点变为 turn34。
协调者独立复跑 WebKit，断言“已加载旧 turn 能在上滚到顶后恢复”失败。

原因链：

- `crates/app-tauri/frontend/src/features/chat/components/chat-stream/index.tsx:2272`
  限制跨 turn 挂载数量，单次恢复 8 个 turn。
- 同文件 `:2469` 将窗口外的旧 turn 切出渲染数组。
- 同文件 `:2951` 先尝试顶部恢复，要求 `!pinnedRef.current`。
- 同文件 `:2969` 随后才根据本次用户滚动取消 pinned。
- 已在顶部时继续向上滚动没有新的 scrollTop 变化，因此没有下一次 scroll
  事件重新执行恢复；wheel 处理只记录手势时间。

建议使本次手势产生的有效 pinned 状态参与顶部恢复判断，或在取消 pinned 后
重新检查顶部恢复条件。保留 landed、smooth-scroll、resize 和位置恢复守卫。
验收应要求首次到顶后开始恢复旧 turn，无需先下滚再上滚。

## P2：虚拟卸载丢失展开状态，返回时正文收起并引起跳动

两引擎均复现同一稳定 step：折叠高度 36px，展开后 72px；
离窗后 placeholder 保留 72px，返回重新挂载后变回 36px。
WebKit 中下一行屏幕位置由 y=145 变为 y=109，移动 36px。
底层 view 内容仍在，丢失的是展开状态。

- `crates/app-tauri/frontend/src/features/chat/components/assistant-status/index.tsx:198`
  将展开状态保存在组件局部 state。
- `crates/app-tauri/frontend/src/features/chat/components/chat-stream/index.tsx:1751`
  在虚拟化时卸载实际内容子树。
- 同文件的高度缓存保留已展开高度，而重新挂载的内容从折叠状态开始。
- `crates/app-tauri/frontend/src/features/chat/components/chat-stream/step-measurements.ts:16`
  只补偿旧底边位于视口上方的高度变化，视口内收缩会移动下方正文。

建议由稳定 session/step 身份持有展开状态，并保持与测量缓存一致。
验收应覆盖离窗超过卸载延迟后返回，详情和相邻正文位置保持。

## 短暂空白与性能：已有观测，归因有限

快速大幅往返时，首个 rAF 可见 7–12 个 placeholder，第三个 rAF 恢复。
该结果证明 DOM 就绪延迟；逐绘制帧空白仍待确认。

综合分页、流式追加、滚轮、切会话和宽度变化场景：

| 引擎 | 帧数 | 最大帧间隔 | 超过 33ms | 超过 70ms |
| --- | ---: | ---: | ---: | ---: |
| Chromium | 1001 | 83.3ms | 11 | 1 |
| WebKit | 997 | 141ms | 80 | 16 |

测量包含探针、DOM 快照和截图开销，不能全部归因为产品代码。
原有单长 turn 测试两引擎均通过，小步往返 drift 为 0；
这一测试覆盖不足以发现上述卡顶及展开状态缺陷。

## 已核实的正常路径与限制

补充分页场景复用真实 store、`viewFromHistoryChunked`、`prependItems`，
只模拟历史页返回，并采用生产的 `key={sessionKey}`。
两个会话均从 historyStart=120 加载到 0，最终各 144 个全局索引连续的 items，
最老用户与助手正文可显示。在途分页期间切会话未观察到跨会话污染。
650px 到原宽度的变化未复现持久正文丢失。

本次使用浏览器开发 fixture 和模拟 IPC；真实 Tauri WKWebView、物理触控板、
真实历史 IPC 及严重卡顿的 CPU 原因仍未验证。
“已加载旧正文被窗口挡住”已确认；当前视口中普通正文自行消失尚未独立复现。

## 建议次序

1. 修复顶部恢复与 pinned 状态更新顺序，建立真实滚轮回归。
2. 修复展开状态的生命周期，使虚拟卸载不改变阅读状态。
3. 对快速滚动的逐帧画面和真实 Tauri 性能做独立测量，再决定挂载调度调整。
