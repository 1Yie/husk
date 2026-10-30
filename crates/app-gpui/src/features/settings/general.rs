use super::*;
use crate::ui::{composer::alpha, icons::Keyline, popover::Menu};
use gpui_kit::component::button::ButtonCustomVariant;
use gpui_kit::component::{menu::PopupMenuItem, switch::Switch};
use gpui_kit::prelude::FluentBuilder as _;
use serde_json::{Value, json};

fn surface(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0x26262d
    } else {
        0xffffff
    })
    .into()
}

fn section(title: &str, description: &str, child: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .w_full()
        .gap(px(12.))
        .child(
            v_flex()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .when(!description.is_empty(), |header| {
                    header.child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(19.5))
                            .text_color(cx.theme().muted_foreground)
                            .child(description.to_owned()),
                    )
                }),
        )
        .child(child)
}

fn card(cx: &App) -> Div {
    v_flex()
        .w_full()
        .rounded(px(22.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(surface(cx))
        .overflow_hidden()
}

fn row(
    icon: Keyline,
    title: &str,
    description: &str,
    control: AnyElement,
    last: bool,
    cx: &App,
) -> Div {
    h_flex()
        .w_full()
        .px(px(20.))
        .py(px(16.))
        .gap(px(16.))
        .flex_wrap()
        .when(!last, |row| {
            row.border_b_1().border_color(alpha(
                if cx.theme().is_dark() {
                    0x2d2d35
                } else {
                    0xf4f4f5
                },
                1.,
            ))
        })
        .child(
            h_flex()
                .flex_1()
                .min_w(px(220.))
                .gap(px(12.))
                .child(
                    div()
                        .size(px(32.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icon.icon()
                                .size(px(16.))
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .child(div().text_size(px(14.)).child(title.to_owned()))
                        .when(!description.is_empty(), |text| {
                            text.child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(19.5))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description.to_owned()),
                            )
                        }),
                ),
        )
        .child(
            div()
                .ml_auto()
                .w(px(200.))
                .min_w_0()
                .flex()
                .justify_end()
                .child(control),
        )
}

fn tinted(color: Hsla, background: Hsla) -> Hsla {
    let fg: Rgba = color.into();
    let bg: Rgba = background.into();
    Rgba {
        r: fg.r * 0.04 + bg.r * 0.96,
        g: fg.g * 0.04 + bg.g * 0.96,
        b: fg.b * 0.04 + bg.b * 0.96,
        a: 1.,
    }
    .into()
}

impl SettingsPanel {
    fn general_select(
        &self,
        id: &str,
        label: String,
        options: Vec<(String, bool, Value)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = !self.pending.is_empty();
        let owner = cx.weak_entity();
        let trigger = Button::new(SharedString::from(format!("general-select:{id}")))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(
                        rgb(if cx.theme().is_dark() {
                            0x2b2b32
                        } else {
                            0xffffff
                        })
                        .into(),
                    )
                    .hover(
                        rgb(if cx.theme().is_dark() {
                            0x2b2b32
                        } else {
                            0xffffff
                        })
                        .into(),
                    )
                    .active(
                        rgb(if cx.theme().is_dark() {
                            0x2b2b32
                        } else {
                            0xffffff
                        })
                        .into(),
                    ),
            )
            .w(px(200.))
            .h(px(32.))
            .px(px(12.))
            .rounded(px(18.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(rgb(if cx.theme().is_dark() {
                0x2b2b32
            } else {
                0xffffff
            }))
            .text_size(px(12.))
            .line_height(px(16.))
            .font_weight(FontWeight::NORMAL)
            .disabled(busy)
            .shadow(vec![BoxShadow {
                color: alpha(0x000000, 0.06),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap(px(8.))
                    .justify_between()
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(
                        Keyline::ChevronsUpDown
                            .icon()
                            .size(px(14.))
                            .text_color(cx.theme().muted_foreground),
                    ),
            );
        Menu::new(
            SharedString::from(format!("general-options:{id}")),
            trigger,
            move |mut menu, _, _| {
                for (label, selected, patch) in options {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(selected)
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |panel, cx| {
                                    panel.settings(SettingsCommand::Preferences(patch.clone()), cx);
                                });
                            }),
                    );
                }
                menu
            },
        )
        .disabled(busy)
        .into_any_element()
    }

    fn general_choice(
        &self,
        key: &'static str,
        current: &Value,
        fallback: &str,
        options: &[(&str, &str)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = current
            .as_str()
            .map(str::to_owned)
            .or_else(|| current.as_u64().map(|value| value.to_string()))
            .unwrap_or_else(|| fallback.to_owned());
        let label = options
            .iter()
            .find(|(value, _)| *value == current)
            .map_or_else(|| current.clone(), |(_, label)| (*label).to_owned());
        let options = options
            .iter()
            .map(|(value, label)| {
                let wire = match key {
                    "approval_timeout_secs" | "question_timeout_secs" => value
                        .parse::<u64>()
                        .ok()
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    "sandbox_max_memory_mb" | "sandbox_max_processes" => value
                        .parse::<u64>()
                        .ok()
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    "compact_at" => value
                        .parse::<f64>()
                        .ok()
                        .map(|value| json!(value / 100.))
                        .unwrap_or(Value::Null),
                    "subagent_thinking_level" if value.is_empty() => Value::Null,
                    _ => Value::String((*value).to_owned()),
                };
                ((*label).to_owned(), *value == current, json!({key:wire}))
            })
            .collect();
        self.general_select(key, label, options, cx)
    }

    fn general_model(&self, data: &Value, child: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = &data["preferences"];
        let (model_key, provider_key, default_label) = if child {
            ("subagent_model", "subagent_provider", "跟随主代理")
        } else {
            ("active_model", "active_provider", "跟随配置默认")
        };
        let mut label = default_label.to_owned();
        let mut options = vec![(
            default_label.to_owned(),
            p[model_key].is_null(),
            json!({model_key:Value::Null, provider_key:Value::Null}),
        )];
        if let Some(providers) = data["config"]["value"]["providers"].as_object() {
            let mut providers = providers.iter().collect::<Vec<_>>();
            providers.sort_by_key(|(name, _)| *name);
            for (provider, config) in providers {
                for model in config["models"].as_array().into_iter().flatten() {
                    let id = model
                        .as_str()
                        .or_else(|| model["id"].as_str())
                        .unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let name = model["name"]
                        .as_str()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or(id);
                    let selected = p[model_key].as_str() == Some(id)
                        && p[provider_key].as_str() == Some(provider.as_str());
                    if selected {
                        label = name.to_owned();
                    }
                    options.push((
                        format!("{name} · {provider}"),
                        selected,
                        json!({model_key:id,provider_key:provider}),
                    ));
                }
            }
        }
        self.general_select(model_key, label, options, cx)
    }

    fn general_switch(
        &self,
        key: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Switch::new(key)
            .checked(checked)
            .disabled(!self.pending.is_empty())
            .on_click(cx.listener(move |panel, checked: &bool, _, cx| {
                panel.settings(SettingsCommand::Preferences(json!({key:checked})), cx);
            }))
            .into_any_element()
    }

    pub(super) fn general_page(&self, data: &Value, cx: &mut Context<Self>) -> AnyElement {
        let p = &data["preferences"];
        if !p.is_object() {
            return div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(
                    data["preferencesError"]
                        .as_str()
                        .unwrap_or("无法读取默认偏好，已禁用编辑以避免覆盖损坏的配置。")
                        .to_owned(),
                )
                .into_any_element();
        }
        let theme = cx.theme().clone();
        let selected = p["permission_mode"].as_str().unwrap_or("auto");
        let busy = !self.pending.is_empty();
        let mut permission = v_flex().w_full().gap(px(12.));
        let specs = [
            (
                "auto",
                "自动",
                "自动执行常规操作，高风险操作需要确认",
                Keyline::Zap,
                0x059669,
            ),
            (
                "bypassPermissions",
                "全部权限",
                "文件修改和终端命令均自动执行",
                Keyline::TriangleAlert,
                0xe11d48,
            ),
            (
                "acceptEdits",
                "仅接受编辑",
                "自动修改文件，终端命令需要确认",
                Keyline::SquarePen,
                0x0284c7,
            ),
            (
                "default",
                "每次确认",
                "文件修改和终端命令均需确认",
                Keyline::ShieldCheck,
                0x7c3aed,
            ),
        ];
        for pair in specs.chunks(2) {
            let mut cards = h_flex().w_full().gap(px(12.)).flex_wrap();
            for &(value, label, description, icon, tone) in pair {
                let active = selected == value;
                let background = if active {
                    tinted(theme.ring, surface(cx))
                } else {
                    surface(cx)
                };
                cards = cards.child(
                    Button::new(SharedString::from(format!("permission-card:{value}")))
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(background)
                                .hover(background)
                                .active(background)
                                .foreground(theme.foreground),
                        )
                        .flex_1()
                        .min_w(px(240.))
                        .h_auto()
                        .p(px(16.))
                        .rounded(px(22.))
                        .border_1()
                        .border_color(if active { theme.ring } else { theme.border })
                        .bg(if active {
                            tinted(theme.ring, surface(cx))
                        } else {
                            surface(cx)
                        })
                        .when(active, |button| {
                            button.shadow(vec![BoxShadow {
                                color: Hsla {
                                    a: 0.25,
                                    ..theme.ring
                                },
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(1.),
                                inset: false,
                            }])
                        })
                        .disabled(busy)
                        .accessibility_label(format!("{label}，{description}"))
                        .child(
                            v_flex()
                                .w_full()
                                .gap(px(12.))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap(px(12.))
                                        .child(
                                            div()
                                                .size(px(32.))
                                                .rounded_full()
                                                .bg(theme.muted)
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    icon.icon().size(px(16.)).text_color(rgb(tone)),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(14.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(label),
                                        )
                                        .when(value == "auto", |line| {
                                            line.child(
                                                div()
                                                    .px(px(6.))
                                                    .py(px(1.))
                                                    .rounded_full()
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .text_size(px(10.))
                                                    .text_color(theme.muted_foreground)
                                                    .child("推荐"),
                                            )
                                        })
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .size(px(16.))
                                                .rounded_full()
                                                .border_1()
                                                .border_color(if active {
                                                    theme.ring
                                                } else {
                                                    theme.border
                                                })
                                                .when(active, |dot| {
                                                    dot.bg(theme.ring)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(
                                                            Keyline::Check
                                                                .icon()
                                                                .size(px(10.))
                                                                .text_color(rgb(0xffffff)),
                                                        )
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(19.5))
                                        .text_color(theme.muted_foreground)
                                        .child(description),
                                ),
                        )
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.settings(
                                SettingsCommand::Preferences(json!({"permission_mode":value})),
                                cx,
                            );
                        })),
                );
            }
            permission = permission.child(cards);
        }
        let levels = [
            ("off", "Off"),
            ("minimal", "Minimal"),
            ("low", "Low"),
            ("medium", "Medium"),
            ("high", "High"),
            ("xhigh", "Xhigh"),
            ("max", "Max"),
        ];
        let mut child_levels = vec![("", "跟随主代理")];
        child_levels.extend(levels);
        let sessions = card(cx)
            .child(row(
                Keyline::File,
                "启动模式",
                "",
                self.general_choice(
                    "default_workspace_mode",
                    &p["default_workspace_mode"],
                    "last",
                    &[("last", "上次使用"), ("build", "编程"), ("office", "工作")],
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Bot,
                "代理模式",
                "",
                self.general_choice(
                    "agent_mode",
                    &p["agent_mode"],
                    "build",
                    &[("build", "构建"), ("plan", "计划"), ("goal", "目标")],
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Cpu,
                "默认模型",
                "",
                self.general_model(data, false, cx),
                false,
                cx,
            ))
            .child(row(
                Keyline::Bot,
                "子代理模型",
                "委派任务默认使用的模型，子代理自身指定的模型优先",
                self.general_model(data, true, cx),
                false,
                cx,
            ))
            .child(row(
                Keyline::Sparkles,
                "子代理思考等级",
                "委派任务的思考强度，子代理自身指定的等级优先",
                self.general_choice(
                    "subagent_thinking_level",
                    &p["subagent_thinking_level"],
                    "",
                    &child_levels,
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Sparkles,
                "思考深度",
                "",
                self.general_choice(
                    "thinking_level",
                    &p["thinking_level"],
                    "medium",
                    &levels,
                    cx,
                ),
                true,
                cx,
            ));
        let timeouts = [
            ("60", "1 分钟"),
            ("120", "2 分钟"),
            ("300", "5 分钟"),
            ("600", "10 分钟"),
            ("1800", "30 分钟"),
            ("0", "永不"),
        ];
        let waits = card(cx)
            .child(row(
                Keyline::Hourglass,
                "审批超时",
                "等待确认的最长时间",
                self.general_choice(
                    "approval_timeout_secs",
                    &p["approval_timeout_secs"],
                    "300",
                    &timeouts,
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Question,
                "提问超时",
                "等待回答的最长时间",
                self.general_choice(
                    "question_timeout_secs",
                    &p["question_timeout_secs"],
                    "600",
                    &timeouts,
                    cx,
                ),
                true,
                cx,
            ));
        let compact = json!((p["compact_at"].as_f64().unwrap_or(0.8) * 100.).round() as u64);
        let context = card(cx)
            .child(row(
                Keyline::Archive,
                "自动压缩",
                "上下文达到此比例时压缩历史消息",
                self.general_choice(
                    "compact_at",
                    &compact,
                    "80",
                    &[("70", "70%"), ("80", "80%"), ("90", "90%")],
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Layers,
                "架构蓝图",
                "向智能体提供项目的模块与依赖信息",
                self.general_switch(
                    "blueprint_enabled",
                    p["blueprint_enabled"].as_bool().unwrap_or(true),
                    cx,
                ),
                true,
                cx,
            ));
        let notifications = card(cx)
            .child(row(
                Keyline::Bell,
                "系统通知",
                "在桌面显示任务进度与待处理提醒",
                self.general_switch(
                    "desktop_notifications",
                    p["desktop_notifications"].as_bool().unwrap_or(true),
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Volume,
                "提示音",
                "任务结束、等待确认或出错时播放提示音",
                self.general_switch(
                    "notification_sound",
                    p["notification_sound"].as_bool().unwrap_or(true),
                    cx,
                ),
                true,
                cx,
            ));
        let sandbox = card(cx)
            .child(row(
                Keyline::ShieldCheck,
                "启用沙盒",
                "隔离命令执行环境",
                self.general_switch(
                    "sandbox_enabled",
                    p["sandbox_enabled"].as_bool().unwrap_or(false),
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Globe,
                "网络访问",
                "",
                self.general_choice(
                    "sandbox_network",
                    &p["sandbox_network"],
                    "auto",
                    &[
                        ("auto", "自动"),
                        ("allow", "始终允许"),
                        ("deny", "始终禁止"),
                    ],
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Cpu,
                "内存上限",
                "",
                self.general_choice(
                    "sandbox_max_memory_mb",
                    &p["sandbox_max_memory_mb"],
                    "default",
                    &[
                        ("default", "默认（2048 MB）"),
                        ("512", "512 MB"),
                        ("1024", "1 GB"),
                        ("4096", "4 GB"),
                        ("8192", "8 GB"),
                    ],
                    cx,
                ),
                false,
                cx,
            ))
            .child(row(
                Keyline::Activity,
                "进程数上限",
                "",
                self.general_choice(
                    "sandbox_max_processes",
                    &p["sandbox_max_processes"],
                    "default",
                    &[
                        ("default", "默认（256）"),
                        ("64", "64"),
                        ("128", "128"),
                        ("512", "512"),
                        ("1024", "1024"),
                    ],
                    cx,
                ),
                true,
                cx,
            ));
        v_flex()
            .w_full()
            .gap(px(32.))
            .child(section("默认权限模式", "用于新建会话", permission, cx))
            .child(section("启动与会话", "", sessions, cx))
            .child(section("等待超时", "", waits, cx))
            .child(section("上下文", "", context, cx))
            .child(section("通知", "", notifications, cx))
            .child(section("沙盒", "", sandbox, cx))
            .into_any_element()
    }
}
