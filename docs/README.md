# 文档索引

本项目遵循 `docs-engineering`。本文件是文档位置与分类的 profile，不重复描述模块行为。

## 当前文档

- 根 `README.md`：项目入口、构建与运行方式。
- 各包的 `README.md`：包契约、使用方式、验证入口和能力边界。
- `.blueprint/<模块>.md`：当前模块设计与依赖边界。

当前文档只陈述现状；实现或约束改变时直接改写相应事实。同一事实只在一个位置维护，
其余位置使用链接；代码或配置已有的事实引用其来源，不再复制一份清单。

## 记录

`docs/research/` 保存调研资料。调研记录描述其日期时的发现，不随系统现状改写。
`docs/proposals/` 保存请求变更的记录，按 `proposed/`、`implemented/`、`archived/` 分状态。
`docs/` 中已有的 handoff、summary 等文件保持现有路径；再次涉及其内容时先判断它属于
当前文档还是记录，不为统一布局批量搬迁。

当前提案：

- [`windows-native-backdrop`](./proposals/proposed/2026-11-01-windows-native-backdrop/proposal.md)：在 Windows 11 上启用内嵌外框与原生窗外材质。
- [`agent-activity`](./proposals/proposed/2026-11-01-agent-activity/proposal.md)：todo 增加进行中状态与一层子任务、子代理可后台运行、工具调用排队时即显示卡片。
- [`status-strip`](./proposals/proposed/2026-11-01-status-strip/proposal.md)：标题栏统计区改成可配置的状态栏，模块可开关、可拖动排序，并补 todo/子代理/权限/成本模块。

新增指南、参考、ADR 或提案时，先按 `docs-engineering` 确认分类和位置，再扩展此索引。
ADR 与已加载的代理指令仍须经所有者批准后修改。

## 工作区与维护

草稿、探针和临时验证材料放在 Git 忽略的 `.husk/scratch/`，不作为项目事实来源。
正式文档不引用临时文件、会话记录或工作区材料。
