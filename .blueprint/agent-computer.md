+++
owns = ["crates/agent-computer/**"]
forbidden_deps = ["agent-kernel"]
+++
桌面操作后端：截图与合成输入（X11 / macOS / Windows）。

关键词：screenshot, mouse, keyboard, desktop control, computer use, x11

## 职责
- `DesktopBackend`：截图、点击/拖拽/滚动、键入、启动应用
- 平台检测与回退（`none` 后端显式报错，不静默成功）

## 不做
- 不决定何时操作桌面、不做权限判断（属于 kernel 的 computer 工具）

## 对外契约
- `DesktopBackend` trait；截图落盘并返回元数据，长边缩放且从不放大
