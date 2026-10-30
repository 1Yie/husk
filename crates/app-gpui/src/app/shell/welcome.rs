use super::*;

impl Shell {
    pub(super) fn workspace_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (greeting, icon, tone) = crate::ui::greeting::time_greeting(theme.is_dark());
        let mut content = v_flex()
            .w_full()
            .max_w(px(720.))
            .items_center()
            .gap(px(32.))
            .child(
                v_flex()
                    .items_center()
                    .gap_5()
                    .child(
                        h_flex()
                            .gap_2p5()
                            .items_center()
                            .child(icon.icon().size(px(20.)).text_color(rgb(tone)))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{greeting}，今天想做点什么？")),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("welcome-open-workspace")
                                    .primary()
                                    .rounded_full()
                                    .px_6()
                                    .h(px(40.))
                                    .icon(Keyline::FolderOpen.icon())
                                    .label("打开工作区…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_workspace(&OpenWorkspace, window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("或从左侧「项目」打开最近的工作区"),
                            ),
                    ),
            );
        if let Some(snapshot) = &self.snapshot {
            let recents = snapshot
                .workspaces
                .iter()
                .filter(|workspace| {
                    std::path::Path::new(&workspace.path) != snapshot.config.data_dir.join("office")
                })
                .take(6)
                .collect::<Vec<_>>();
            if !recents.is_empty() {
                let mut grid = h_flex().w_full().gap_2().flex_wrap();
                for workspace in recents {
                    let path = PathBuf::from(&workspace.path);
                    grid = grid.child(
                        Button::new(SharedString::from(format!(
                            "welcome-recent:{}",
                            workspace.path
                        )))
                        .ghost()
                        .w(px(350.))
                        .min_w_0()
                        .h_auto()
                        .py_3()
                        .px_3()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(theme.border)
                        .justify_start()
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_3()
                                .child(Keyline::FolderOpen.icon().size(px(18.)))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .truncate()
                                                .child(workspace.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .truncate()
                                                .child(workspace.path.clone()),
                                        ),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(Command::SwitchWorkspace(path.clone()), true, cx)
                        })),
                    );
                }
                content = content.child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("最近工作区"),
                        )
                        .child(grid),
                );
            }
        }
        let mut capabilities = h_flex().w_full().gap_2().flex_wrap();
        for (icon, title, description) in [
            (Keyline::Code, "代码读写", "读取、搜索和修改项目文件"),
            (Keyline::Zap, "沙箱与审批", "命令在沙箱内运行，写操作需批准"),
            (Keyline::File, "多会话", "切换会话时进行中的任务继续运行"),
            (
                Keyline::Terminal,
                "命令与技能",
                "`/` 命令、`$` 技能、MCP 插件",
            ),
        ] {
            capabilities = capabilities.child(
                v_flex()
                    .w(px(174.))
                    .gap_1()
                    .p_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(icon.icon().size(px(14.)).text_color(theme.muted_foreground))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(description),
                    ),
            );
        }
        content = content.child(capabilities).child(
            h_flex()
                .gap_3()
                .text_xs()
                .child(
                    Button::new("welcome-docs")
                        .ghost()
                        .xsmall()
                        .label("使用文档")
                        .on_click(|_, _, cx| cx.open_url("https://husk.ingstar.im")),
                )
                .child(
                    Button::new("welcome-source")
                        .ghost()
                        .xsmall()
                        .label("源代码")
                        .on_click(|_, _, cx| cx.open_url("https://github.com/1Yie/husk")),
                ),
        );
        div()
            .id("workspace-welcome")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px_6()
            .py_10()
            .child(content)
            .into_any_element()
    }
}
