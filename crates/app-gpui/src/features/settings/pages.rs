use super::*;
use gpui_kit::component::{button::ButtonVariants, switch::Switch};
use serde_json::{Value, json};

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn section(title: &str, description: &str) -> Div {
    v_flex()
        .gap_3()
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(div().text_sm().child(description.to_owned()))
}
fn note(value: impl Into<String>) -> Div {
    div().text_sm().child(value.into())
}
impl SettingsPanel {
    fn choices(
        &self,
        id: &'static str,
        title: &str,
        selected: &str,
        items: &[(&str, &str)],
        appearance: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut controls = h_flex().gap_2().flex_wrap();
        let busy = !self.pending.is_empty();
        for (index, (value, label)) in items.iter().enumerate() {
            let value = value.to_string();
            let key = id.to_owned();
            controls = controls.child(
                Button::new((id, index))
                    .small()
                    .outline()
                    .disabled(busy)
                    .label(format!(
                        "{}{}",
                        if selected == value { "✓ " } else { "" },
                        label
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut patch = json!({});
                        patch[&key] = json!(value);
                        this.settings(
                            if appearance {
                                SettingsCommand::Appearance(patch)
                            } else {
                                SettingsCommand::Preferences(patch)
                            },
                            cx,
                        );
                    })),
            );
        }
        section(title, "").child(controls).into_any_element()
    }
    fn toggle(
        &self,
        id: &'static str,
        title: &str,
        description: &str,
        checked: bool,
        available: bool,
        command: SettingsCommand,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_4()
            .justify_between()
            .child(
                v_flex().gap_1().child(title.to_owned()).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(description.to_owned()),
                ),
            )
            .child(
                Switch::new(id)
                    .checked(checked)
                    .disabled(!available || !self.pending.is_empty())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.settings(command.clone(), cx)),
                    ),
            )
            .into_any_element()
    }
    fn models(
        &self,
        data: &Value,
        key: &'static str,
        title: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prefs = &data["preferences"];
        let providers = &data["config"]["value"]["providers"];
        let mut row = h_flex().gap_2().flex_wrap();
        let busy = !self.pending.is_empty();
        let provider_key = if key == "active_model" {
            "active_provider"
        } else {
            "subagent_provider"
        };
        row = row.child(
            Button::new((key, 0usize))
                .small()
                .outline()
                .disabled(busy)
                .label(if prefs[key].is_null() {
                    "✓ 跟随配置 / 父代理"
                } else {
                    "跟随配置 / 父代理"
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let mut p = json!({});
                    p[key] = Value::Null;
                    p[provider_key] = Value::Null;
                    this.settings(SettingsCommand::Preferences(p), cx);
                })),
        );
        let mut index = 1usize;
        if let Some(providers) = providers.as_object() {
            let mut providers = providers.iter().collect::<Vec<_>>();
            providers.sort_by_key(|(id, _)| *id);
            for (provider, value) in providers {
                if let Some(models) = value["models"].as_array() {
                    for model in models {
                        let id = model
                            .as_str()
                            .unwrap_or_else(|| text(model, "id"))
                            .to_owned();
                        let provider = provider.clone();
                        let selected = prefs[key] == id && prefs[provider_key] == provider;
                        row = row.child(
                            Button::new((key, index))
                                .small()
                                .outline()
                                .disabled(busy)
                                .label(format!(
                                    "{}{} / {}",
                                    if selected { "✓ " } else { "" },
                                    provider,
                                    id
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut patch = json!({});
                                    patch[key] = json!(id);
                                    patch[provider_key] = json!(provider);
                                    this.settings(SettingsCommand::Preferences(patch), cx);
                                })),
                        );
                        index += 1;
                    }
                }
            }
        }
        section(title, "用于新建会话，不改变当前会话的运行模型。")
            .child(row)
            .into_any_element()
    }
    pub(super) fn render_page(
        &mut self,
        snapshot: &Snapshot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let data = &snapshot.model_info["settings"];
        let p = &data["preferences"];
        let busy = !self.pending.is_empty();
        if data.is_null() {
            return note("设置尚未加载。点击刷新从宿主读取。").into_any_element();
        }
        let mut page = v_flex().gap_7().w_full();
        match self.page {
            SettingsPage::General => return self.general_page(data, cx),
            SettingsPage::Appearance => {
                let a = &data["appearance"];
                page=page.child(self.choices("theme_mode","主题",text(a,"theme_mode"),&[("system","跟随系统"),("light","浅色"),("dark","深色")],true,cx)).child(self.choices("currency","货币单位",text(a,"currency"),&[("usd","美元 USD"),("cny","人民币 CNY")],true,cx))
                    .child(section("减少动态效果","自动遵循系统的减少动态效果偏好。宿主未提供独立覆盖设置，不会写入虚构字段。").child(Button::new("reduce-motion").small().disabled(true).label("跟随系统")));
            }
            SettingsPage::Model => {
                let config = &data["config"];
                if let Some(error) = config["error"].as_str() {
                    return section("无法读取模型配置", error)
                        .child(note("已禁止编辑，避免用空配置覆盖现有文件。"))
                        .into_any_element();
                }
                page = page.child(
                    section(
                        "提供商与模型",
                        "管理 API 接入与模型参数。密钥不会进入设置快照，留空保持已保存密钥。",
                    )
                    .child(
                        Button::new("provider-add")
                            .outline()
                            .label("＋ 添加提供商")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.provider_editor(
                                    None,
                                    json!({"kind":"openai_completions","base_url":"","api_key":"","models":[]}),
                                    window,
                                    cx,
                                )
                            })),
                    ),
                );
                if let Some(providers) = config["value"]["providers"].as_object() {
                    let mut rows = providers.iter().collect::<Vec<_>>();
                    rows.sort_by_key(|(id, _)| *id);
                    for (index, (id, value)) in rows.into_iter().enumerate() {
                        let provider = id.clone();
                        let edit_id = provider.clone();
                        let edit_value = value.clone();
                        let remove = provider.clone();
                        let add = provider.clone();
                        let mut card = section(
                            id,
                            &format!("{} · {}", text(value, "kind"), text(value, "base_url")),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new(("provider-edit", index))
                                        .small()
                                        .outline()
                                        .label("编辑提供商")
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.provider_editor(
                                                Some(edit_id.clone()),
                                                edit_value.clone(),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(("provider-delete", index))
                                        .small()
                                        .ghost()
                                        .label("删除提供商")
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.confirm(
                                                format!("删除提供商 {remove} 及其模型？"),
                                                SettingsCommand::DeleteProvider(remove.clone()),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(("model-add", index))
                                        .small()
                                        .outline()
                                        .label("＋ 添加模型")
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.model_editor(
                                                add.clone(),
                                                None,
                                                json!({"id":"","input":["text"]}),
                                                window,
                                                cx,
                                            )
                                        })),
                                ),
                        );
                        if let Some(models) = value["models"].as_array() {
                            for (mi, model) in models.iter().enumerate() {
                                let value = if model.is_string() {
                                    json!({"id":model.as_str().unwrap_or_default(),"input":["text"]})
                                } else {
                                    model.clone()
                                };
                                let id = text(&value, "id").to_owned();
                                let label = text(&value, "name");
                                let edit_id = id.clone();
                                let edit_value = value.clone();
                                let edit_provider = provider.clone();
                                let remove_id = id.clone();
                                let remove_provider = provider.clone();
                                card = card.child(
                                    h_flex()
                                        .justify_between()
                                        .gap_3()
                                        .child(
                                            v_flex()
                                                .child(if label.is_empty() {
                                                    id.clone()
                                                } else {
                                                    format!("{label} · {id}")
                                                })
                                                .child(note(format!(
                                                    "上下文 {} · 最大输出 {}",
                                                    value["context_window"], value["max_tokens"]
                                                ))),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new(("model-edit", index * 10000 + mi))
                                                        .small()
                                                        .ghost()
                                                        .label("编辑")
                                                        .disabled(busy)
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.model_editor(
                                                                    edit_provider.clone(),
                                                                    Some(edit_id.clone()),
                                                                    edit_value.clone(),
                                                                    window,
                                                                    cx,
                                                                )
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    Button::new((
                                                        "model-delete",
                                                        index * 10000 + mi,
                                                    ))
                                                    .small()
                                                    .ghost()
                                                    .label("删除")
                                                    .disabled(busy)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.confirm(
                                                                format!("删除模型 {remove_id}？"),
                                                                SettingsCommand::DeleteModel {
                                                                    provider: remove_provider
                                                                        .clone(),
                                                                    id: remove_id.clone(),
                                                                },
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                                ),
                                        ),
                                );
                            }
                        }
                        page = page.child(card);
                    }
                }
                let chain = config["value"]["fallback_chain"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                page = page.child(
                    section(
                        "提供商回退链",
                        "遇到不可恢复的限流或服务错误时，按顺序尝试下一提供商。",
                    )
                    .child(note(if chain.is_empty() {
                        "未设置".to_owned()
                    } else {
                        chain.clone()
                    }))
                    .child(
                        Button::new("fallback-edit")
                            .small()
                            .outline()
                            .disabled(busy)
                            .label("编辑回退链")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit(
                                    "提供商回退链",
                                    EditorAction::FallbackChain,
                                    vec![FieldSpec::text(
                                        "providers",
                                        "提供商 ID（按顺序，逗号分隔；留空禁用）",
                                        chain.clone(),
                                    )],
                                    window,
                                    cx,
                                )
                            })),
                    ),
                );
                page = page.child(note(format!("配置文件：{}", text(data, "config_path"))));
            }
            SettingsPage::Memory => {
                let memory = snapshot.config.memory_preferences;
                let enabled = memory.is_some_and(|m| m.enabled);
                let distill = memory.is_some_and(|m| m.distill);
                // Preserve stable automation IDs and fail-closed preference reads.
                page=page.child(h_flex().justify_between().child(section("长期记忆","记录偏好、事实和对话经验。更改主开关与蒸馏只影响新建会话。")) .child(Switch::new("memory-enabled").checked(enabled).disabled(busy||memory.is_none()).on_click(cx.listener(move |this,_,_,cx|{this.send(Command::SetMemoryEnabled(!enabled),cx);}))))
                    .child(h_flex().justify_between().child(section("模型蒸馏","关闭后仍写入确定性事实，但不调用模型总结。")) .child(Switch::new("memory-distill").checked(distill).disabled(busy||memory.is_none()).on_click(cx.listener(move |this,_,_,cx|{this.send(Command::SetMemoryDistill(!distill),cx);}))))
                    .child(self.toggle("semantic-recall","语义召回","本地编码器可能需要下载模型。",p["semantic_recall"].as_bool().unwrap_or(false),data["semantic"]["compiled"].as_bool().unwrap_or(false)&&!p.is_null(),SettingsCommand::Preferences(json!({"semantic_recall":!p["semantic_recall"].as_bool().unwrap_or(false)})),cx));
                let semantic = &data["semantic"];
                page = page.child(
                    section(
                        "语义模型",
                        &format!(
                            "{} · {}",
                            text(semantic, "name"),
                            if semantic["ready"].as_bool().unwrap_or(false) {
                                "就绪"
                            } else if semantic["cached"].as_bool().unwrap_or(false) {
                                "已缓存"
                            } else {
                                "未下载"
                            }
                        ),
                    )
                    .child(note(
                        serde_json::to_string(&semantic["progress"]).unwrap_or_default(),
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("semantic-retry")
                                    .small()
                                    .outline()
                                    .label("重试下载")
                                    .disabled(
                                        busy || !semantic["compiled"].as_bool().unwrap_or(false),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings(SettingsCommand::SemanticRetry, cx)
                                    })),
                            )
                            .child(
                                Button::new("semantic-cancel")
                                    .small()
                                    .ghost()
                                    .label("取消下载")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings(SettingsCommand::SemanticCancel, cx)
                                    })),
                            )
                            .child(
                                Button::new("semantic-remove")
                                    .small()
                                    .ghost()
                                    .label("卸载模型")
                                    .disabled(
                                        busy || !semantic["cached"].as_bool().unwrap_or(false),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(
                                            "卸载本地语义模型？".into(),
                                            SettingsCommand::SemanticRemove,
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    ),
                );
                let memory = &data["memory"];
                if let Some(error) = memory["error"].as_str() {
                    page = page.child(note(format!("记忆不可用：{error}")));
                } else {
                    page = page.child(
                        section("偏好与习惯", &format!("{} 条经验", memory["episodes"])).child(
                            Button::new("memory-add")
                                .outline()
                                .label("＋ 添加偏好")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.memory_editor(json!({"scope":"global"}), window, cx)
                                })),
                        ),
                    );
                    if let Some(rows) = memory["persona"].as_array() {
                        for (i, row) in rows.iter().enumerate() {
                            let edit = row.clone();
                            let key = text(row, "key").to_owned();
                            let scope = text(row, "scope").to_owned();
                            page = page.child(
                                h_flex()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .child(format!("{} · {}", key, scope))
                                            .child(note(text(row, "value"))),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new(("memory-edit", i))
                                                    .small()
                                                    .ghost()
                                                    .label("编辑")
                                                    .disabled(busy)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.memory_editor(
                                                                edit.clone(),
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new(("memory-forget", i))
                                                    .small()
                                                    .ghost()
                                                    .label("忘记")
                                                    .disabled(busy)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.confirm(
                                                                format!("忘记 {key}？"),
                                                                SettingsCommand::MemoryForget {
                                                                    scope: scope.clone(),
                                                                    key: key.clone(),
                                                                },
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            ),
                                    ),
                            );
                        }
                    }
                    page = page.child(section("事实", "最近 200 条事实；置信度由记忆系统维护。"));
                    if let Some(rows) = memory["facts"].as_array() {
                        for (i, row) in rows.iter().enumerate() {
                            let id = row["id"].as_u64().unwrap_or(0);
                            page = page.child(
                                h_flex()
                                    .justify_between()
                                    .gap_3()
                                    .child(note(format!(
                                        "{} · 置信度 {}",
                                        text(row, "text"),
                                        row["confidence"]
                                    )))
                                    .child(
                                        Button::new(("fact-forget", i))
                                            .small()
                                            .ghost()
                                            .label("忘记")
                                            .disabled(busy)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.confirm(
                                                    "忘记这条事实？".into(),
                                                    SettingsCommand::MemoryRemoveFact(id),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    ),
                            );
                        }
                    }
                    let mut clear = h_flex().gap_2().flex_wrap();
                    for (i, (table, label)) in [
                        ("persona", "清空偏好"),
                        ("facts", "清空事实"),
                        ("episodes", "清空经验"),
                        ("all", "清空记忆"),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        clear = clear.child(
                            Button::new(("memory-clear", i))
                                .small()
                                .outline()
                                .disabled(busy)
                                .label(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm(
                                        format!("{label}？"),
                                        SettingsCommand::MemoryClear(table.into()),
                                        window,
                                        cx,
                                    )
                                })),
                        );
                    }
                    page = page.child(clear);
                }
            }
            SettingsPage::Instructions => {
                page = page.child(section(
                    "指令",
                    "全局与工作区 AGENTS.md 会在后续提示中生效。",
                ));
                for (i, (scope, label)) in [("global", "全局指令"), ("workspace", "工作区指令")]
                    .into_iter()
                    .enumerate()
                {
                    let value = &data["instructions"][scope];
                    let content = text(value, "content").to_owned();
                    let shown = content.clone();
                    page = page.child(
                        section(label, text(value, "path"))
                            .child(note(if shown.is_empty() {
                                "尚无指令".to_owned()
                            } else {
                                shown
                            }))
                            .child(
                                Button::new(("instructions-edit", i))
                                    .small()
                                    .outline()
                                    .label("编辑")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit(
                                            label,
                                            EditorAction::Instructions(scope.into()),
                                            vec![FieldSpec::long(
                                                "content",
                                                "Markdown 指令",
                                                content.clone(),
                                            )],
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    );
                }
            }
            SettingsPage::Rules
            | SettingsPage::Subagent
            | SettingsPage::Skills
            | SettingsPage::Mcp => {
                let kind = self.page;
                let key = match kind {
                    SettingsPage::Rules => "rules",
                    SettingsPage::Subagent => "subagents",
                    SettingsPage::Skills => "skills",
                    _ => "mcp",
                };
                let description = match kind {
                    SettingsPage::Rules => "常驻约束与文件匹配规则。工作区规则覆盖同名全局规则。",
                    SettingsPage::Subagent => {
                        "管理内置与自定义子代理。自定义同名代理覆盖内置代理。"
                    }
                    SettingsPage::Skills => {
                        "管理 .pi/skills 下的全局和工作区技能。其他安装目录的发现尚未接入本机桥接。"
                    }
                    _ => "管理全局 MCP stdio / HTTP 服务器。保存后新建会话生效；连接探测尚未接入。",
                };
                page = page.child(
                    section(self.page.title(), description).child(
                        Button::new("extension-add")
                            .outline()
                            .label("＋ 新建")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.extension_editor(kind, json!({}), window, cx)
                            })),
                    ),
                );
                if let Some(rows) = data[key].as_array() {
                    for (i, row) in rows.iter().enumerate() {
                        let edit = row.clone();
                        let name = if kind == SettingsPage::Mcp {
                            text(row, "id")
                        } else {
                            text(row, "name")
                        }
                        .to_owned();
                        let global = row["global"].as_bool().unwrap_or(false);
                        let builtin = row["builtin"].as_bool().unwrap_or(false);
                        let remove_name = name.clone();
                        let command = match kind {
                            SettingsPage::Rules => SettingsCommand::DeleteRule {
                                name: remove_name,
                                global,
                            },
                            SettingsPage::Subagent => SettingsCommand::DeleteSubagent {
                                name: remove_name,
                                global,
                            },
                            SettingsPage::Skills => SettingsCommand::DeleteSkill {
                                name: remove_name,
                                global,
                            },
                            _ => SettingsCommand::DeleteMcp(remove_name),
                        };
                        let mut controls = h_flex().gap_2().child(
                            Button::new(("extension-edit", i))
                                .small()
                                .outline()
                                .label(if builtin { "自定义覆盖" } else { "编辑" })
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.extension_editor(kind, edit.clone(), window, cx)
                                })),
                        );
                        if !builtin {
                            controls = controls.child(
                                Button::new(("extension-delete", i))
                                    .small()
                                    .ghost()
                                    .label("删除")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm(
                                            format!("删除 {name}？"),
                                            command.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            );
                        }
                        page = page.child(
                            section(
                                if kind == SettingsPage::Mcp {
                                    text(row, "id")
                                } else {
                                    text(row, "name")
                                },
                                text(row, "description"),
                            )
                            .child(note(if builtin {
                                "内置"
                            } else if global {
                                "全局"
                            } else {
                                "工作区"
                            }))
                            .child(note(text(row, "error")))
                            .child(controls),
                        );
                    }
                }
            }
            SettingsPage::Plugins => {
                page = page.child(
                    section(
                        "插件",
                        "已发现的插件、信任与启用状态。信任前请核对插件代码。",
                    )
                    .child(
                        Button::new("plugins-reload")
                            .outline()
                            .label("重新加载")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings(SettingsCommand::PluginReload, cx)
                            })),
                    ),
                );
                if let Some(rows) = data["plugins"].as_array() {
                    if rows.is_empty() {
                        page = page.child(note("尚未发现插件。插件安装服务尚未接入此原生宿主。"));
                    }
                    for (i, row) in rows.iter().enumerate() {
                        let id = text(row, "id").to_owned();
                        let enabled = row["enabled"].as_bool().unwrap_or(false);
                        let toggle_id = id.clone();
                        let trust = id.clone();
                        let remove = id.clone();
                        page = page.child(
                            section(&id, text(row, "description"))
                                .child(note(format!(
                                    "状态：{} · {}",
                                    if enabled { "启用" } else { "禁用" },
                                    row["error"]
                                )))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Switch::new(("plugin-enabled", i))
                                                .checked(enabled)
                                                .disabled(busy)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.settings(
                                                        SettingsCommand::PluginEnabled {
                                                            id: toggle_id.clone(),
                                                            enabled: !enabled,
                                                        },
                                                        cx,
                                                    )
                                                })),
                                        )
                                        .child(
                                            Button::new(("plugin-trust", i))
                                                .small()
                                                .outline()
                                                .label("信任")
                                                .disabled(
                                                    busy || row["trusted"]
                                                        .as_bool()
                                                        .unwrap_or(false),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.confirm(
                                                            format!("信任插件 {trust}？"),
                                                            SettingsCommand::PluginTrust(
                                                                trust.clone(),
                                                            ),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new(("plugin-remove", i))
                                                .small()
                                                .ghost()
                                                .label("删除")
                                                .disabled(busy)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.confirm(
                                                            format!("删除插件 {remove}？"),
                                                            SettingsCommand::PluginRemove(
                                                                remove.clone(),
                                                            ),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                )),
                                        ),
                                ),
                        );
                    }
                }
            }
            SettingsPage::Terminal => {
                let preferences = &data["terminal"];
                let unavailable = busy || !preferences.is_object();
                if let Some(error) = data["terminalError"].as_str() {
                    page = page.child(note(format!(
                        "终端设置读取失败，已禁用编辑且不会覆盖原文件：{error}"
                    )));
                }
                let mut fonts = vec![("默认等宽字体".to_owned(), json!(""))];
                fonts.extend(
                    _window
                        .text_system()
                        .all_font_names()
                        .into_iter()
                        .filter(|name| {
                            let name = name.to_lowercase();
                            ["mono", "code", "console", "courier"]
                                .iter()
                                .any(|part| name.contains(part))
                        })
                        .map(|name| (name.clone(), json!(name))),
                );
                let rows: Vec<(&str, &str, Vec<(String, Value)>)> = vec![
                    (
                        "scheme",
                        "配色方案",
                        [
                            ("跟随应用", "husk"),
                            ("GitHub Light", "github-light"),
                            ("GitHub Dark", "github-dark"),
                            ("One Dark", "one-dark"),
                            ("Solarized Light", "solarized-light"),
                            ("Dracula", "dracula"),
                            ("Nord", "nord"),
                        ]
                        .into_iter()
                        .map(|(label, value)| (label.to_owned(), json!(value)))
                        .collect(),
                    ),
                    ("fontFamily", "字体", fonts),
                    (
                        "fontSize",
                        "字号",
                        (10..=24)
                            .map(|n| (n.to_string(), json!(n as f32)))
                            .collect(),
                    ),
                    (
                        "lineHeight",
                        "行高",
                        (10..=20)
                            .map(|n| (format!("{:.1}", n as f32 / 10.), json!(n as f32 / 10.)))
                            .collect(),
                    ),
                    (
                        "cursorStyle",
                        "光标形态",
                        [("竖线", "bar"), ("方块", "block"), ("下划线", "underline")]
                            .into_iter()
                            .map(|(label, value)| (label.to_owned(), json!(value)))
                            .collect(),
                    ),
                ];
                for (key, title, options) in rows {
                    let key = key.to_owned();
                    let selected = preferences[&key].clone();
                    let label = options
                        .iter()
                        .find(|(_, value)| *value == selected)
                        .map(|(label, _)| label.clone())
                        .unwrap_or_else(|| selected.as_str().unwrap_or("未加载").to_owned());
                    let entity = cx.entity().downgrade();
                    page = page.child(
                        section(title, "").child(
                            crate::ui::popover::Menu::new(
                                SharedString::from(format!("terminal-menu-{key}")),
                                Button::new(format!("terminal-menu-trigger-{key}"))
                                    .outline()
                                    .disabled(unavailable)
                                    .label(label),
                                move |mut menu, _, _| {
                                    for (label, value) in &options {
                                        let entity = entity.clone();
                                        let key = key.clone();
                                        let value = value.clone();
                                        menu =
                                            menu.item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    label.clone(),
                                                )
                                                .checked(selected == value)
                                                .disabled(unavailable)
                                                .on_click(move |_, _, cx| {
                                                    let mut patch = json!({});
                                                    patch[&key] = value.clone();
                                                    let _ = entity.update(cx, |this, cx| {
                                                        this.settings(
                                                            SettingsCommand::Terminal(patch),
                                                            cx,
                                                        )
                                                    });
                                                }),
                                            );
                                    }
                                    menu
                                },
                            )
                            .disabled(unavailable),
                        ),
                    );
                }
                page = page.child(
                    section("光标闪烁", "应用可通过 ANSI 光标命令覆盖此默认值。").child(
                        Switch::new("terminal-cursor-blink")
                            .checked(preferences["cursorBlink"].as_bool().unwrap_or(true))
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.settings(
                                    SettingsCommand::Terminal(json!({"cursorBlink": checked})),
                                    cx,
                                );
                            })),
                    ),
                );
                for (title, reason) in [
                    (
                        "回滚行数",
                        "原生终端沿用受限的安全历史缓冲区，暂不支持运行时调整。",
                    ),
                    (
                        "正常退出后关闭标签",
                        "当前保留已退出终端的内容，暂不支持自动关闭标签。",
                    ),
                ] {
                    page = page.child(
                        section(title, reason).child(
                            Button::new(format!("terminal-unsupported-{title}"))
                                .small()
                                .outline()
                                .disabled(true)
                                .label("暂不支持"),
                        ),
                    );
                }
                page = page.child(note("外观设置实时应用于已有终端，不重建 PTY。字体菜单列出本机已安装的等宽候选；未安装字符由系统回退。分栏与标签保持不变。"));
            }
            SettingsPage::Stats => {
                page = page.child(section(
                    "使用统计",
                    "读取宿主持久化的会话用量，不包含推测数据。",
                ));
                if let Some(rows) = data["stats"].as_array() {
                    if rows.is_empty() {
                        page = page.child(note("暂无用量记录"));
                    }
                    let total =
                        |key: &str| rows.iter().filter_map(|r| r[key].as_u64()).sum::<u64>();
                    page = page.child(
                        h_flex()
                            .gap_6()
                            .flex_wrap()
                            .child(section("输入 tokens", &total("prompt").to_string()))
                            .child(section("输出 tokens", &total("completion").to_string()))
                            .child(section("缓存命中", &total("cached").to_string()))
                            .child(section("缓存写入", &total("cache_write").to_string())),
                    );
                    for row in rows {
                        page = page.child(
                            section(
                                &format!("{} · {}", text(row, "workspace"), text(row, "title")),
                                &format!("{} / {}", text(row, "provider"), text(row, "model")),
                            )
                            .child(note(format!(
                                "输入 {} · 输出 {} · 缓存命中 {} · 缓存写入 {}",
                                row["prompt"], row["completion"], row["cached"], row["cache_write"]
                            )))
                            .child(note(format!(
                                "蒸馏调用 {} · 蒸馏输入 {} · 蒸馏输出 {}",
                                row["distill_calls"],
                                row["distill_prompt"],
                                row["distill_completion"]
                            ))),
                        );
                    }
                }
            }
            SettingsPage::Developer => {
                page = page.child(
                    section(
                        "开发者选项",
                        "Tauri WebView 渲染器配置不适用于 GPUI。此宿主未暴露对应设置 API。",
                    )
                    .child(note(format!(
                        "原生后端：GPUI\n平台：{} / {}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ))),
                );
            }
            SettingsPage::About => {
                page = page
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Husk"),
                    )
                    .child(note(format!(
                        "版本 {} · GPUI 原生桌面端",
                        env!("CARGO_PKG_VERSION")
                    )))
                    .child(section("了解 Husk", "基于 Rust 的智能体工作空间。"));
                for (i, (label, url)) in [
                    ("官网", "https://husk.ingstar.im"),
                    ("使用文档", "https://husk.ingstar.im/docs"),
                    ("源代码", "https://github.com/1Yie/husk"),
                    ("Korei 内核", "https://github.com/1Yie/korei"),
                    ("反馈问题", "https://github.com/1Yie/husk/issues"),
                    ("发行版本", "https://github.com/1Yie/husk/releases"),
                ]
                .into_iter()
                .enumerate()
                {
                    page = page.child(
                        Button::new(("about-link", i))
                            .outline()
                            .label(label)
                            .on_click(move |_, _, cx| cx.open_url(url)),
                    );
                }
                page = page.child(section(
                    "版本更新",
                    "原生宿主尚未接入更新检查服务。可通过发行版本页手动查看。",
                ));
                page = page.child(section(
                    "本地数据",
                    &format!(
                        "配置：{}\n数据：{}\n工作区：{}",
                        snapshot.config.config_dir.display(),
                        snapshot.config.data_dir.display(),
                        text(data, "workspace")
                    ),
                ));
            }
        }
        page.into_any_element()
    }
    fn provider_editor(
        &mut self,
        original: Option<String>,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let empty_headers = json!({});
        let fields = vec![
            FieldSpec::text("id", "提供商 ID", original.clone().unwrap_or_default()),
            FieldSpec::text("name", "显示名称", text(&value, "name")),
            FieldSpec::choice(
                "kind",
                "API 类型",
                text(&value, "kind"),
                &[
                    ("openai_completions", "OpenAI Chat Completions"),
                    ("openai_responses", "OpenAI Responses"),
                    ("openai_codex_responses", "OpenAI Codex Responses"),
                    ("anthropic", "Anthropic"),
                    ("gemini", "Google Gemini"),
                ],
            ),
            FieldSpec::text("base_url", "API 地址", text(&value, "base_url")),
            FieldSpec::secret("api_key", "API 密钥（留空保留，可使用 env:变量名）"),
            FieldSpec::text(
                "default_model",
                "默认模型 ID（可选）",
                text(&value, "default_model"),
            ),
            FieldSpec::text(
                "user_agent",
                "User-Agent（可选）",
                text(&value, "user_agent"),
            ),
            FieldSpec::json(
                "headers",
                "请求头 JSON（遮罩保持现有值）",
                if value["headers"].is_null() {
                    &empty_headers
                } else {
                    &value["headers"]
                },
            ),
            FieldSpec::json("compat", "兼容性 JSON（null 使用默认）", &value["compat"]),
        ];
        self.edit(
            if original.is_some() {
                "编辑提供商"
            } else {
                "添加提供商"
            },
            EditorAction::Provider {
                original,
                base: value,
            },
            fields,
            window,
            cx,
        );
    }
    fn model_editor(
        &mut self,
        provider: String,
        original: Option<String>,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = vec![
            FieldSpec::text("id", "模型 ID", text(&value, "id")),
            FieldSpec::text("name", "显示名称", text(&value, "name")),
            FieldSpec::number(
                "context_window",
                "上下文窗口（留空继承）",
                &value["context_window"],
            ),
            FieldSpec::number(
                "max_tokens",
                "最大输出 tokens（留空继承）",
                &value["max_tokens"],
            ),
            FieldSpec::boolean(
                "reasoning",
                "推理模型（开关）",
                value["reasoning"].as_bool().unwrap_or(false),
            ),
            FieldSpec::json(
                "input",
                "输入模态 JSON（如 [\"text\", \"image\"]）",
                value.get("input").unwrap_or(&json!(["text"])),
            ),
            FieldSpec::json("cost", "计费 JSON（null 不设置）", &value["cost"]),
            FieldSpec::json(
                "sampling_params",
                "采样参数 JSON（null 不设置）",
                &value["sampling_params"],
            ),
            FieldSpec::json(
                "fastMode",
                "Fast 模式 JSON（null 不设置）",
                &value["fastMode"],
            ),
        ];
        self.edit(
            if original.is_some() {
                "编辑模型"
            } else {
                "添加模型"
            },
            EditorAction::Model {
                provider,
                original,
                base: value,
            },
            fields,
            window,
            cx,
        );
    }
    fn memory_editor(&mut self, value: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(
            "记忆偏好",
            EditorAction::Memory,
            vec![
                FieldSpec::text(
                    "scope",
                    "作用域（global / workspace）",
                    text(&value, "scope"),
                ),
                FieldSpec::text("key", "名称", text(&value, "key"))
                    .locked(!text(&value, "key").is_empty()),
                FieldSpec::long("value", "内容", text(&value, "value")),
            ],
            window,
            cx,
        );
    }
    fn extension_editor(
        &mut self,
        kind: SettingsPage,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let global = value["global"].as_bool().unwrap_or(false);
        let name = text(&value, "name");
        let (action, fields) = match kind {
            SettingsPage::Rules => (
                EditorAction::Rule,
                vec![
                    FieldSpec::text("name", "名称", name).locked(!name.is_empty()),
                    FieldSpec::text("description", "说明", text(&value, "description")),
                    FieldSpec::boolean("global", "全局作用域（开关）", global),
                    FieldSpec::boolean(
                        "always",
                        "始终加载（开关）",
                        value["always"].as_bool().unwrap_or(true),
                    ),
                    FieldSpec::text(
                        "globs",
                        "文件匹配（逗号分隔）",
                        value["globs"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default(),
                    ),
                    FieldSpec::long("body", "规则内容", text(&value, "body")),
                ],
            ),
            SettingsPage::Subagent => (
                EditorAction::Subagent,
                vec![
                    FieldSpec::text("name", "名称", name).locked(!name.is_empty()),
                    FieldSpec::text("description", "说明", text(&value, "description")),
                    FieldSpec::boolean("global", "全局作用域（开关）", global),
                    FieldSpec::boolean(
                        "readonly",
                        "只读（开关）",
                        value["readonly"].as_bool().unwrap_or(false),
                    ),
                    FieldSpec::text("model", "模型覆盖（可选）", text(&value, "model")),
                    FieldSpec::text("thinking", "思考等级覆盖（可选）", text(&value, "thinking")),
                    FieldSpec::long("prompt", "系统提示", text(&value, "prompt")),
                ],
            ),
            SettingsPage::Skills => (
                EditorAction::Skill,
                vec![
                    FieldSpec::text("name", "目录名称", name).locked(!name.is_empty()),
                    FieldSpec::boolean("global", "全局作用域（开关）", global),
                    FieldSpec::long(
                        "content",
                        "SKILL.md（含 YAML frontmatter）",
                        text(&value, "content"),
                    ),
                ],
            ),
            _ => (
                EditorAction::Mcp,
                vec![
                    FieldSpec::text("id", "服务器 ID", text(&value, "id"))
                        .locked(!text(&value, "id").is_empty()),
                    FieldSpec::json(
                        "manifest",
                        "MCP 配置 JSON（command / args 或 url；遮罩保持凭据）",
                        value
                            .get("manifest")
                            .unwrap_or(&json!({"command":"","args":[]})),
                    ),
                ],
            ),
        };
        self.edit(kind.title(), action, fields, window, cx);
    }
}
