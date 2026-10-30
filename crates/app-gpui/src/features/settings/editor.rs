use super::*;
use gpui_kit::component::{
    input::{AnyInputState, Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use serde_json::{Value, json};

#[derive(Clone)]
pub(super) enum EditorAction {
    Provider {
        original: Option<String>,
        base: Value,
    },
    Model {
        provider: String,
        original: Option<String>,
        base: Value,
    },
    Instructions(String),
    Rule,
    Subagent,
    Skill,
    Mcp,
    Memory,
    FallbackChain,
}
#[derive(Clone, Copy)]
pub(super) enum FieldKind {
    Text,
    Secret,
    Multiline,
    Json,
    Number,
    Bool,
    Choice(&'static [(&'static str, &'static str)]),
}
pub(super) struct FieldSpec {
    key: &'static str,
    label: &'static str,
    value: String,
    kind: FieldKind,
    locked: bool,
}
impl FieldSpec {
    pub fn choice(
        key: &'static str,
        label: &'static str,
        value: impl Into<String>,
        options: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            key,
            label,
            value: value.into(),
            kind: FieldKind::Choice(options),
            locked: false,
        }
    }
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
    pub fn text(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            key,
            label,
            value: value.into(),
            locked: false,
            kind: FieldKind::Text,
        }
    }
    pub fn long(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            key,
            label,
            value: value.into(),
            locked: false,
            kind: FieldKind::Multiline,
        }
    }
    pub fn json(key: &'static str, label: &'static str, value: &Value) -> Self {
        Self {
            key,
            label,
            value: serde_json::to_string_pretty(value).unwrap_or_default(),
            locked: false,
            kind: FieldKind::Json,
        }
    }
    pub fn number(key: &'static str, label: &'static str, value: &Value) -> Self {
        Self {
            key,
            label,
            value: value.as_u64().map(|n| n.to_string()).unwrap_or_default(),
            locked: false,
            kind: FieldKind::Number,
        }
    }
    pub fn boolean(key: &'static str, label: &'static str, value: bool) -> Self {
        Self {
            key,
            label,
            value: value.to_string(),
            locked: false,
            kind: FieldKind::Bool,
        }
    }
    pub fn secret(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            value: String::new(),
            locked: false,
            kind: FieldKind::Secret,
        }
    }
}
struct Field {
    spec: FieldSpec,
    state: AnyInputState,
}
pub(super) struct Editor {
    action: EditorAction,
    fields: Vec<Field>,
    pub error: Option<String>,
    pub pending: bool,
}
impl Editor {
    pub fn new(
        action: EditorAction,
        fields: Vec<FieldSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields = fields
            .into_iter()
            .map(|spec| {
                let state =
                    if matches!(spec.kind, FieldKind::Multiline | FieldKind::Json) {
                        AnyInputState::Textarea(cx.new(|cx| {
                            TextareaState::new(window, cx).default_value(spec.value.clone())
                        }))
                    } else {
                        AnyInputState::Input(cx.new(|cx| {
                            InputState::new(window, cx)
                                .default_value(spec.value.clone())
                                .masked(matches!(spec.kind, FieldKind::Secret))
                        }))
                    };
                Field { spec, state }
            })
            .collect();
        Self {
            action,
            fields,
            error: None,
            pending: false,
        }
    }
    pub fn command(&self, cx: &App) -> Result<SettingsCommand, String> {
        let mut values = json!({});
        for field in &self.fields {
            let raw = field.state.value(cx).to_string();
            values[field.spec.key] = match field.spec.kind {
                FieldKind::Json => {
                    serde_json::from_str(&raw).map_err(|e| format!("{}：{e}", field.spec.label))?
                }
                FieldKind::Number => {
                    if raw.trim().is_empty() {
                        Value::Null
                    } else {
                        json!(
                            raw.parse::<u64>()
                                .map_err(|_| format!("{}必须为非负整数", field.spec.label))?
                        )
                    }
                }
                FieldKind::Bool => json!(
                    raw.parse::<bool>()
                        .map_err(|_| format!("{}必须为 true 或 false", field.spec.label))?
                ),
                _ => json!(raw),
            };
        }
        let string = |key: &str| values[key].as_str().unwrap_or("").to_owned();
        let global = values["global"].as_bool().unwrap_or(false);
        Ok(match &self.action {
            EditorAction::Provider { original, base } => {
                let id = string("id");
                let mut value = base.clone();
                for key in [
                    "kind",
                    "base_url",
                    "api_key",
                    "name",
                    "default_model",
                    "user_agent",
                ] {
                    value[key] = values[key].clone();
                }
                for key in ["name", "default_model", "user_agent"] {
                    if value[key]
                        .as_str()
                        .is_some_and(|value| value.trim().is_empty())
                    {
                        value[key] = Value::Null;
                    }
                }
                value["headers"] = values["headers"].clone();
                value["compat"] = values["compat"].clone();
                SettingsCommand::SaveProvider {
                    original: original.clone(),
                    id,
                    value,
                }
            }
            EditorAction::Model {
                provider,
                original,
                base,
            } => {
                let mut value = base.clone();
                for key in [
                    "id",
                    "name",
                    "context_window",
                    "max_tokens",
                    "reasoning",
                    "input",
                    "cost",
                    "sampling_params",
                    "fastMode",
                ] {
                    value[key] = values[key].clone();
                }
                SettingsCommand::SaveModel {
                    provider: provider.clone(),
                    original: original.clone(),
                    value,
                }
            }
            EditorAction::Instructions(scope) => SettingsCommand::Instructions {
                scope: scope.clone(),
                content: string("content"),
            },
            EditorAction::Rule => {
                let globs = string("globs")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                values["globs"] = json!(globs);
                SettingsCommand::SaveRule(values)
            }
            EditorAction::Subagent => SettingsCommand::SaveSubagent(values),
            EditorAction::Skill => SettingsCommand::SaveSkill {
                name: string("name"),
                global,
                content: string("content"),
            },
            EditorAction::Mcp => SettingsCommand::SaveMcp {
                id: string("id"),
                value: values["manifest"].clone(),
            },
            EditorAction::FallbackChain => SettingsCommand::FallbackChain(
                string("providers")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            EditorAction::Memory => SettingsCommand::MemorySet {
                scope: string("scope"),
                key: string("key"),
                value: string("value"),
            },
        })
    }
}
impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("settings-editor")
            .gap_3()
            .max_h(px(540.))
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        for (index, field) in self.fields.iter().enumerate() {
            let input = match &field.state {
                AnyInputState::Input(state) if matches!(field.spec.kind, FieldKind::Choice(_)) => {
                    let FieldKind::Choice(options) = field.spec.kind else {
                        unreachable!()
                    };
                    let value = state.read(cx).value().to_string();
                    let label = options
                        .iter()
                        .find(|(key, _)| *key == value)
                        .map(|(_, label)| (*label).to_owned())
                        .unwrap_or_else(|| value.clone());
                    let state = state.clone();
                    let disabled = self.pending || field.spec.locked;
                    crate::ui::popover::Menu::new(
                        ("editor-choice", index),
                        Button::new(("editor-choice-trigger", index))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .label(label)
                            .disabled(disabled),
                        move |mut menu, _, _| {
                            for (key, label) in options {
                                let state = state.clone();
                                menu = menu.item(
                                    gpui_kit::component::menu::PopupMenuItem::new(*label)
                                        .checked(*key == value)
                                        .disabled(disabled)
                                        .on_click(move |_, window, cx| {
                                            state.update(cx, |state, cx| {
                                                state.set_value(*key, window, cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        },
                    )
                    .disabled(disabled)
                    .into_any_element()
                }
                AnyInputState::Input(state) if matches!(field.spec.kind, FieldKind::Bool) => {
                    let checked = state.read(cx).value().as_ref() == "true";
                    let state = state.clone();
                    Switch::new(("editor-switch", index))
                        .checked(checked)
                        .disabled(self.pending || field.spec.locked)
                        .on_click(move |checked, window, cx| {
                            state.update(cx, |state, cx| {
                                state.set_value(checked.to_string(), window, cx)
                            })
                        })
                        .into_any_element()
                }
                AnyInputState::Input(state) => Input::new(state)
                    .disabled(self.pending)
                    .readonly(field.spec.locked)
                    .into_any_element(),
                AnyInputState::Textarea(state) => Textarea::new(state)
                    .h(px(120.))
                    .disabled(self.pending)
                    .into_any_element(),
                _ => div().into_any_element(),
            };
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(field.spec.label))
                    .child(input),
            );
        }
        if self.pending {
            body = body.child("正在保存，等待宿主确认…");
        }
        body
    }
}
