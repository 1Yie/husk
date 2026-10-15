# 前端测试

前端使用 Vitest 3、React Testing Library、user-event 和 jsdom。测试与业务源码放在一起，公共测试工具位于 `src/test/`。不需要启动 Rust 后端、Tauri 窗口或模型服务。

## 运行

在 `crates/app-tauri/frontend` 中执行：

```sh
npm ci
npm test                 # 单次运行全部测试
npm run test:watch       # 本地监听改动
npm run test:ui          # Vitest 交互界面
npm run test:coverage    # 覆盖率报告与关键模块门槛
npm run typecheck        # 检查应用、测试、测试工具与配置的类型
npm run check            # 类型检查、覆盖率测试、生产构建
```

运行单个模块或用例：

```sh
npm test -- src/stores/__tests__/agent-store.test.tsx
npm test -- -t 'stale restored tier'
```

Bun 也可以用 `bun run test` / `bun run test:watch` 调用相同的 Vitest 配置。请勿使用 `bun test`：本项目的测试运行器是 Vitest。CI 和发布使用 npm；依赖变更须同步 `package-lock.json` 与 `bun.lock`。

## 目录与测试层次

- `src/**/*.test.ts`：档位校验、导航分页边界、事件 reducer、历史回放、IPC 协议。
- `src/**/*.test.tsx`：真实 store 订阅、组件交互、思考与输入栏生命周期，以及事件到界面的完整对话流程。
- `src/test/setup.ts`：jest-dom 断言、DOM 能力安装、Tauri 模块模拟，以及每例结束后的清理。
- `src/test/tauri.ts`：集中管理 native invoke、事件监听、响应与乱序 Promise。
- `src/test/dom.ts`：可手动触发的尺寸/可见性观察器；不伪造浏览器的布局计算结果。
- `src/test/render.tsx`：公共 Provider 和组件渲染函数。

## Tauri 与事件边界

业务模块保持真实，只在原生能力边界模拟 `invoke`、`listen` 和窗口 API。未配置的 native 命令默认抛错，避免空对象响应掩盖缺失的测试依赖。每个用例可以覆写命令响应：

```ts
import { mockCommand, modelInfo, emitAgentEvent } from '@/test/tauri';

mockCommand('agent_session', () => ({
  ...modelInfo(), active_thinking_level: 'xhigh',
}), 'model_info');

emitAgentEvent({
  root: '/project', session: 1,
  event: { ReasoningDelta: { text: '分析边界', parent: null } },
});
```

`deferred<T>()` 用来控制并发请求的返回顺序。用它验证旧模型响应不会覆盖新选择，避免真实网络或依赖 sleep 的竞态测试。

store 的事件订阅通过 `initAgentStore()` 安装，返回 disposer；集成测试在结束后必须调用它。它会释放 native 监听、store 订阅、配置事件和未提交的合并定时器。测试中用 `useAgentStore.getInitialState()` 重置状态，不需要重置模块或引入另一份 React。

## 定时器、DOM 与组件

事件合并和耗时标签使用假定时器，并在 React `act()` 中推进。用户输入、菜单和审批使用 Testing Library 按角色/可访问名称查找，避免整页快照和依赖类名的断言。

思考状态容器测试只替换重 Markdown 子组件，真实保留状态容器和 Radix 交互，因此仍能捕获跨 16k 字符时的节点卸载问题。对话流程测试同样只隔离 Markdown/高亮，事件、reducer、store、ChatStream 和 ComposerBar 均走真实实现。

jsdom 不计算真实排版，也不模拟 GPU、CSS 动画性能、Tauri WebView 或网络延迟。本套件验证业务状态、生命周期与交互；帧率、滚动几何和原生兼容性应另用浏览器或 Tauri 实测。不要在这些测试中用固定毫秒数声称性能通过。

## 覆盖率与 CI

`npm run test:coverage` 输出整个 `src` 的覆盖率，包含尚未覆盖的页面，避免只报告已测模块而掩盖缺口。思考档位、历史导航边界和输入栏更新判定设置了独立门槛；后续新增测试可逐步扩大门槛范围，不宣称整个前端已经覆盖完整。

CI 的 frontend job 执行：

1. `npm ci`
2. `npm run typecheck`
3. `npm run test:ci`（测试、V8 覆盖率、JUnit）
4. `npm run build`

`coverage/`（HTML、LCOV、JSON）和 `test-results/`（JUnit）已被 Git 忽略；CI 将其上传为保留 7 天的报告，测试夹具和正式用例才进入版本管理。
