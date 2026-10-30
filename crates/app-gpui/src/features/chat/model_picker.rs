use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::bridge::{Command, ConfigInfo, HostBridge, SessionKey};
use crate::ui::icons::Keyline;
use crate::ui::popover::Popover;

const TIERS: &[(&str, &str, &str)] = &[
    ("off", "Off", "Off"),
    ("minimal", "Min", "Minimal"),
    ("low", "Low", "Low"),
    ("medium", "Med", "Medium"),
    ("high", "High", "High"),
    ("xhigh", "Xhigh", "Xhigh"),
    ("max", "Max", "Max"),
];

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("")
}

fn model_label(model: &Value) -> &str {
    let name = text(model, "name").trim();
    if name.is_empty() {
        text(model, "model")
    } else {
        name
    }
}

fn tier_label(value: &str) -> &str {
    TIERS
        .iter()
        .find(|tier| tier.0 == value)
        .map_or(value, |tier| tier.2)
}

fn tier_token(value: &str) -> &str {
    TIERS
        .iter()
        .find(|tier| tier.0 == value)
        .map_or(value, |tier| tier.1)
}

/// Search, drag previews and errors belong to this entity, not the transcript.
/// ChatPanel retains it, including while the popover animates in and out.
pub(super) struct ModelPicker {
    key: SessionKey,
    bridge: Arc<HostBridge>,
    info: Value,
    level: String,
    levels: Vec<String>,
    preview: usize,
    search: Entity<InputState>,
    slider: Entity<SliderState>,
    pending: Option<u64>,
    error: Option<String>,
    _search_subscription: Subscription,
    slider_subscription: Option<Subscription>,
}

impl ModelPicker {
    pub(super) fn new(
        key: SessionKey,
        bridge: Arc<HostBridge>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索模型或提供商…"));
        let subscription = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            key,
            bridge,
            info: Value::Null,
            level: "off".into(),
            levels: Vec::new(),
            preview: 0,
            search,
            slider: cx.new(|_| SliderState::new().min(0.).max(1.).step(1.)),
            pending: None,
            error: None,
            _search_subscription: subscription,
            slider_subscription: None,
        }
    }

    fn active_model(&self) -> Option<&Value> {
        self.info["models"].as_array()?.iter().find(|model| {
            model["provider"] == self.info["active_provider"]
                && model["model"] == self.info["active_model"]
        })
    }

    fn fast_available(&self) -> bool {
        self.active_model()
            .and_then(|model| model["supports_fast_mode"].as_bool())
            .unwrap_or(false)
    }

    fn fast_enabled(&self) -> bool {
        self.fast_available() && self.info["active_fast_mode"].as_bool().unwrap_or(false)
    }

    pub(super) fn set_context(
        &mut self,
        info: &Value,
        config: &ConfigInfo,
        cx: &mut Context<Self>,
    ) {
        self.info = info.clone();
        let active = self.active_model();
        let declared = active
            .filter(|model| model["reasoning"].as_bool() != Some(false))
            .and_then(|model| model["available_levels"].as_array());
        let levels: Vec<String> = TIERS
            .iter()
            .filter(|tier| {
                declared
                    .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(tier.0)))
                    && !active.is_some_and(|model| {
                        model["thinking_level_map"]
                            .as_object()
                            .and_then(|map| map.get(tier.0))
                            .is_some_and(Value::is_null)
                    })
            })
            .map(|tier| tier.0.to_owned())
            .collect();
        let requested = config
            .thinking_level
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let level = levels
            .iter()
            .find(|level| **level == requested)
            .or_else(|| levels.iter().find(|level| *level == "medium"))
            .or_else(|| levels.iter().find(|level| *level != "off"))
            .cloned()
            .unwrap_or_else(|| "off".into());
        if self.levels != levels || self.level != level {
            self.levels = levels;
            self.level = level;
            self.sync_slider(cx);
        }
        cx.notify();
    }

    fn sync_slider(&mut self, cx: &mut Context<Self>) {
        self.preview = self
            .levels
            .iter()
            .position(|level| level == &self.level)
            .unwrap_or(0);
        self.slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(self.levels.len().saturating_sub(1).max(1) as f32)
                .step(1.)
                .default_value(self.preview as f32)
        });
        self.slider_subscription = Some(cx.subscribe(&self.slider, |this, _, event, cx| {
            if this.pending.is_some() || this.levels.is_empty() {
                return;
            }
            match event {
                SliderEvent::Change(SliderValue::Single(value)) => {
                    this.preview = (*value as usize).min(this.levels.len() - 1);
                    cx.notify();
                }
                SliderEvent::Release(SliderValue::Single(value)) => {
                    let index = (*value as usize).min(this.levels.len() - 1);
                    this.select_level(index, cx);
                }
                _ => {}
            }
        }));
    }

    fn select_level(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let Some(level) = self.levels.get(index).cloned() else {
            return;
        };
        if level == self.level {
            return;
        }
        self.send(
            Command::SetThinkingLevel {
                key: self.key.clone(),
                level,
            },
            cx,
        );
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        match self.bridge.send(command) {
            Ok(id) => self.pending = Some(id),
            Err(error) => {
                self.error = Some(error);
                self.sync_slider(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn complete(
        &mut self,
        request_id: u64,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending != Some(request_id) {
            return false;
        }
        self.pending = None;
        self.error = error.map(str::to_owned);
        self.sync_slider(cx);
        cx.notify();
        true
    }

    pub(super) fn control(entity: &Entity<Self>, cx: &App) -> Popover {
        let picker = entity.read(cx);
        let model = picker
            .active_model()
            .map(model_label)
            .unwrap_or_else(|| text(&picker.info, "active_model"));
        let label = if model.is_empty() {
            "选择模型"
        } else {
            model
        };
        let muted = cx.theme().muted_foreground;
        let content = entity.clone();
        let opened = entity.downgrade();
        Popover::new(SharedString::from(format!(
            "model-thinking-picker:{}:{}",
            picker.key.root, picker.key.id,
        )))
        .anchor(Anchor::BottomRight)
        .offset(px(4.))
        .duration(crate::ui::motion::MENU_DURATION)
        .p(px(4.))
        .trigger(
            Button::new("model-thinking-trigger")
                .ghost()
                .small()
                .h(px(28.))
                .px(px(10.))
                .rounded(px(14.))
                .text_size(px(12.))
                .line_height(px(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if cx.theme().is_dark() {
                    rgb(0xa1a1aa)
                } else {
                    rgb(0x52525b)
                })
                .tooltip(format!("当前模型: {label}"))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .min_w_0()
                        .child(div().max_w(px(180.)).truncate().child(label.to_owned()))
                        .when(!picker.levels.is_empty(), |row| {
                            row.child(
                                h_flex()
                                    .gap(px(4.))
                                    .pl(px(6.))
                                    .border_l_1()
                                    .border_color(crate::ui::composer::alpha(
                                        if cx.theme().is_dark() {
                                            0x2d2d35
                                        } else {
                                            0xe4e4e7
                                        },
                                        0.7,
                                    ))
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(Keyline::Brain.icon().size(px(12.)).text_color(
                                        if picker.level == "off" {
                                            muted
                                        } else {
                                            rgb(if cx.theme().is_dark() {
                                                0xc084fc
                                            } else {
                                                0x9333ea
                                            })
                                            .into()
                                        },
                                    ))
                                    .child(tier_token(&picker.level).to_owned()),
                            )
                        })
                        .when(picker.fast_enabled(), |row| {
                            row.child(Keyline::Zap.icon().size(px(12.)).text_color(rgb(
                                if cx.theme().is_dark() {
                                    0xc084fc
                                } else {
                                    0x9333ea
                                },
                            )))
                        })
                        .child(
                            Keyline::ChevronsUpDown
                                .icon()
                                .size(px(14.))
                                .text_color(muted),
                        ),
                ),
        )
        .on_open_change(move |open, window, cx| {
            if *open {
                let _ = opened.update(cx, |picker, cx| {
                    picker.search.update(cx, |search, cx| {
                        search.set_value("", window, cx);
                        search.focus(window, cx);
                    });
                });
            }
        })
        .content(move |_, _, _| content)
    }
}

impl Render for ModelPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let foreground = theme.foreground;
        let selected_color = theme.success;
        let pending = self.pending.is_some();
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        if let Some(models) = self.info["models"].as_array() {
            for model in models {
                if text(model, "provider").is_empty() || text(model, "model").is_empty() {
                    continue;
                }
                let haystack = format!(
                    "{} {} {} {}",
                    text(model, "provider"),
                    model_label(model),
                    text(model, "model"),
                    text(model, "description")
                )
                .to_lowercase();
                if query.split_whitespace().all(|term| haystack.contains(term)) {
                    groups
                        .entry(text(model, "provider").into())
                        .or_default()
                        .push(model.clone());
                }
            }
        }
        let mut list = v_flex()
            .id("model-picker-results")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(2.));
        if groups.is_empty() {
            list = list.child(div().p(px(12.)).text_center().text_color(muted).child(
                if query.is_empty() {
                    "未检测到模型"
                } else {
                    "没有匹配的模型"
                },
            ));
        }
        for (group_index, (provider, models)) in groups.into_iter().enumerate() {
            list = list
                .when(group_index > 0, |list| {
                    list.child(div().h(px(1.)).my(px(4.)).bg(border))
                })
                .child(
                    div()
                        .px(px(8.))
                        .py(px(5.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(provider),
                );
            for model in models {
                let provider = text(&model, "provider").to_owned();
                let id = text(&model, "model").to_owned();
                let active = self.info["active_provider"].as_str() == Some(&provider)
                    && self.info["active_model"].as_str() == Some(&id);
                let label = model_label(&model).to_owned();
                let description = text(&model, "description").to_owned();
                let context = model["context_window"]
                    .as_u64()
                    .or_else(|| model["contextWindow"].as_u64());
                let context_label = context.map(|size| {
                    if size >= 1_000_000 {
                        format!("{:.1}M context", size as f64 / 1_000_000.)
                    } else if size >= 1_000 {
                        format!("{}K context", size / 1_000)
                    } else {
                        format!("{size} context")
                    }
                });
                let mut details = v_flex().flex_1().min_w_0().gap(px(2.));
                // Keep the wire id subordinate; a nameless model prints it only once.
                details = details.child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .child(label.clone()),
                );
                if label != id {
                    details = details.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(id.clone()),
                    );
                }
                if !description.is_empty() {
                    details = details.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(description.clone()),
                    );
                }
                if let Some(context) = context_label {
                    details =
                        details.child(div().text_size(px(10.)).text_color(muted).child(context));
                }
                let tooltip = if description.is_empty() {
                    format!("{provider}/{id}")
                } else {
                    format!("{provider}/{id}\n{description}")
                };
                let row_id = SharedString::from(format!("model-picker-row:{provider}/{id}"));
                list = list.child(
                    Button::new(row_id)
                        .ghost()
                        .w_full()
                        .h_auto()
                        .px(px(8.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .disabled(pending)
                        .tooltip(tooltip)
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap(px(8.))
                                .child(details)
                                .child(
                                    div()
                                        .w(px(14.))
                                        .text_color(selected_color)
                                        .child(if active { "✓" } else { "" }),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.pending.is_some() {
                                return;
                            }
                            let exists = this.info["models"].as_array().is_some_and(|models| {
                                models.iter().any(|model| {
                                    text(model, "provider") == provider
                                        && text(model, "model") == id
                                })
                            });
                            if !exists
                                || (text(&this.info, "active_provider") == provider
                                    && text(&this.info, "active_model") == id)
                            {
                                return;
                            }
                            this.send(
                                Command::SetModel {
                                    key: this.key.clone(),
                                    provider: provider.clone(),
                                    model: id.clone(),
                                },
                                cx,
                            );
                        })),
                );
            }
        }

        let fast_available = self.fast_available();
        let preview = self.levels.get(self.preview).unwrap_or(&self.level).clone();
        let mapped = self
            .active_model()
            .and_then(|model| model["thinking_level_map"][&preview].as_str())
            .filter(|mapped| *mapped != preview && !mapped.is_empty())
            .map(str::to_owned);
        let mut footer = v_flex()
            .px(px(8.))
            .pt(px(7.))
            .pb(px(5.))
            .gap(px(4.))
            .border_t_1()
            .border_color(border)
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child("思考")
                            .child(
                                div()
                                    .text_color(muted)
                                    .text_size(px(11.))
                                    .child(tier_label(&preview).to_owned()),
                            )
                            .when_some(mapped, |row, mapped| {
                                row.child(
                                    div()
                                        .text_color(muted)
                                        .text_size(px(10.))
                                        .child(format!("→ {mapped}")),
                                )
                            }),
                    )
                    .when(fast_available, |row| {
                        row.child(
                            h_flex()
                                .gap(px(5.))
                                .child(Keyline::Zap.icon().size(px(13.)).text_color(
                                    if self.fast_enabled() {
                                        theme.primary
                                    } else {
                                        muted
                                    },
                                ))
                                .child(div().text_size(px(11.)).child("Fast"))
                                .child(
                                    Switch::new("model-picker-fast")
                                        .small()
                                        .checked(self.fast_enabled())
                                        .disabled(pending)
                                        .accessibility_label("快速模式")
                                        .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                            if this.pending.is_some() || !this.fast_available() {
                                                return;
                                            }
                                            this.send(
                                                Command::SetFastMode {
                                                    key: this.key.clone(),
                                                    enabled: *enabled,
                                                },
                                                cx,
                                            );
                                        })),
                                ),
                        )
                    }),
            );
        if self.levels.is_empty() {
            footer = footer.child(
                div()
                    .py(px(8.))
                    .text_color(muted)
                    .text_size(px(11.))
                    .child("此模型没有可调思考等级。"),
            );
        } else {
            footer = footer.child(
                Slider::new(&self.slider)
                    .w_full()
                    .disabled(pending || self.levels.len() < 2)
                    .accessibility_label("思考等级"),
            );
            let mut stops = h_flex().w_full().justify_between();
            for (index, level) in self.levels.iter().enumerate() {
                stops = stops.child(
                    Button::new(("thinking-stop", index))
                        .ghost()
                        .small()
                        .h(px(20.))
                        .px(px(2.))
                        .text_size(px(10.))
                        .text_color(if self.preview == index {
                            foreground
                        } else {
                            muted
                        })
                        .label(tier_token(level).to_owned())
                        .disabled(pending)
                        .tooltip(tier_label(level).to_owned())
                        .on_click(cx.listener(move |this, _, _, cx| this.select_level(index, cx))),
                );
            }
            footer = footer.child(stops);
        }
        v_flex()
            .w(px(312.))
            .h(px(312.))
            .text_size(px(12.))
            .child(
                div()
                    .px(px(8.))
                    .py(px(5.))
                    .text_color(muted)
                    .child("模型与思考"),
            )
            .child(
                div().px(px(4.)).pb(px(6.)).child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Keyline::Search.icon().size(px(14.))),
                ),
            )
            .child(div().h(px(1.)).bg(border))
            .child(list)
            .child(footer)
            .when(pending, |view| {
                view.child(
                    div()
                        .px(px(8.))
                        .pb(px(4.))
                        .text_size(px(10.))
                        .text_color(muted)
                        .child("正在应用…"),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px(px(8.))
                        .pb(px(4.))
                        .text_size(px(11.))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
    }
}
