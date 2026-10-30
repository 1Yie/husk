use super::*;
use chrono::{Datelike, Local, TimeZone};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Deserialize)]
struct OfficeFile {
    path: String,
    name: String,
    modified: u64,
}

#[derive(Default)]
pub(super) struct OfficeFiles {
    root: String,
    files: Vec<OfficeFile>,
    error: Option<String>,
    pending: HashSet<u64>,
    latest: u64,
    loaded: bool,
    truncated: bool,
    closed: HashSet<String>,
}

impl Shell {
    pub(super) fn refresh_office_files(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(active) = &snapshot.active else {
            return;
        };
        if std::path::Path::new(&active.root) != snapshot.config.data_dir.join("office") {
            return;
        }
        let root = active.root.clone();
        if self.office.root != root {
            self.office.root = root.clone();
            self.office.files.clear();
            self.office.loaded = false;
            self.office.error = None;
        }
        if self.office.pending.contains(&self.office.latest) {
            return;
        }
        match self.bridge.send(Command::Activity(
            crate::bridge::ActivityCommand::OfficeArtifacts { root },
        )) {
            Ok(id) => {
                self.office.latest = id;
                self.office.pending.insert(id);
                self.office.error = None;
            }
            Err(error) => self.office.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn receive_office_files(
        &mut self,
        id: u64,
        key: &SessionKey,
        data: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if data["kind"].as_str() != Some("office_artifacts") {
            return false;
        }
        if self.office.pending.remove(&id)
            && id == self.office.latest
            && key.root == self.office.root
        {
            match serde_json::from_value(data["artifacts"].clone()) {
                Ok(files) => {
                    self.office.files = files;
                    self.office.error = None;
                    self.office.loaded = true;
                    self.office.truncated = data["truncated"].as_bool().unwrap_or(false);
                }
                Err(error) => self.office.error = Some(error.to_string()),
            }
            cx.notify();
        }
        true
    }

    pub(super) fn fail_office_files(
        &mut self,
        id: u64,
        message: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.office.pending.remove(&id) {
            return false;
        }
        if id == self.office.latest {
            self.office.error = Some(message.to_owned());
            cx.notify();
        }
        true
    }

    pub(super) fn render_office_files(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let mut body = v_flex().w_full().px_2().gap_0p5();
        if let Some(error) = &self.office.error {
            body = body.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        if self.office.files.is_empty() {
            return body
                .child(
                    div()
                        .px_2()
                        .py_3()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if self.office.pending.contains(&self.office.latest) {
                            "正在读取产物…"
                        } else if self.office.loaded {
                            "暂无产物，工作中生成的文件会显示在这里。"
                        } else {
                            "产物将在打开工作区后载入。"
                        }),
                )
                .into_any_element();
        }
        let today = Local::now().date_naive();
        let mut groups: Vec<(String, String, Vec<OfficeFile>)> = Vec::new();
        for file in &self.office.files {
            let date = Local
                .timestamp_opt(file.modified.min(i64::MAX as u64) as i64, 0)
                .single()
                .map(|date| date.date_naive())
                .unwrap_or(today);
            let key = date.to_string();
            if groups.last().is_none_or(|group| group.0 != key) {
                let days = today.signed_duration_since(date).num_days();
                let label = if days <= 0 {
                    "今天".into()
                } else if days == 1 {
                    "昨天".into()
                } else if date.year() == today.year() {
                    format!("{}月{}日", date.month(), date.day())
                } else {
                    format!("{}年{}月{}日", date.year(), date.month(), date.day())
                };
                groups.push((key, label, Vec::new()));
            }
            if let Some(group) = groups.last_mut() {
                group.2.push(file.clone());
            }
        }
        for (day, label, files) in groups {
            let open = !self.office.closed.contains(&day);
            let toggle = day.clone();
            body = body.child(
                Button::new(SharedString::from(format!("office-day:{day}")))
                    .ghost()
                    .w_full()
                    .h(px(32.))
                    .justify_start()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1p5()
                            .child(
                                if open {
                                    Keyline::ChevronDown
                                } else {
                                    Keyline::ChevronRight
                                }
                                .icon()
                                .xsmall(),
                            )
                            .child(
                                if open {
                                    Keyline::FolderOpen
                                } else {
                                    Keyline::Folder
                                }
                                .icon()
                                .small(),
                            )
                            .child(div().flex_1().truncate().child(label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(files.len().to_string()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.office.closed.remove(&toggle) {
                            this.office.closed.insert(toggle.clone());
                        }
                        cx.notify();
                    })),
            );
            let mut types = v_flex().w_full();
            for category in ["PDF", "DOC", "PPT", "XLS", "素材", "其他"] {
                let files = files
                    .iter()
                    .filter(|file| file_category(&file.path) == category)
                    .cloned()
                    .collect::<Vec<_>>();
                if files.is_empty() {
                    continue;
                }
                let key = format!("{day}:{category}");
                let expanded = !self.office.closed.contains(&key);
                let toggle = key.clone();
                types = types.child(
                    Button::new(SharedString::from(format!("office-type:{key}")))
                        .ghost()
                        .w_full()
                        .h(px(32.))
                        .pl_4()
                        .justify_start()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_1p5()
                                .child(
                                    if expanded {
                                        Keyline::ChevronDown
                                    } else {
                                        Keyline::ChevronRight
                                    }
                                    .icon()
                                    .xsmall(),
                                )
                                .child(Keyline::File.icon().small())
                                .child(div().flex_1().text_xs().child(category))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(files.len().to_string()),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.office.closed.remove(&toggle) {
                                this.office.closed.insert(toggle.clone());
                            }
                            cx.notify();
                        })),
                );
                let mut rows = v_flex().w_full();
                for file in files {
                    let path = PathBuf::from(&self.office.root).join(&file.path);
                    let open_path = path.clone();
                    let reveal = path.clone();
                    let copy = path;
                    rows = rows.child(
                        h_flex()
                            .w_full()
                            .gap_0p5()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "office-file:{}",
                                    file.path
                                )))
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .h(px(32.))
                                .pl_8()
                                .justify_start()
                                .tooltip(file.path.clone())
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_1p5()
                                        .child(Keyline::File.icon().small())
                                        .child(
                                            div().min_w_0().truncate().text_xs().child(file.name),
                                        ),
                                )
                                .on_click(move |_, _, cx| {
                                    if let Ok(url) = url::Url::from_file_path(&open_path) {
                                        cx.open_url(url.as_str());
                                    }
                                }),
                            )
                            .child(Menu::new(
                                SharedString::from(format!("office-file-menu:{}", file.path)),
                                Button::new("office-file-more")
                                    .ghost()
                                    .xsmall()
                                    .icon(Keyline::More.icon()),
                                move |menu, _, _| {
                                    menu.item(
                                        PopupMenuItem::new("在文件夹中显示")
                                            .icon(Keyline::FolderOpen.icon())
                                            .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                                    )
                                    .item(
                                        PopupMenuItem::new("复制路径")
                                            .icon(Keyline::Copy.icon())
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy.to_string_lossy().into_owned(),
                                                ))
                                            }),
                                    )
                                },
                            )),
                    );
                }
                let progress = gpui_kit::base::motion::transition(
                    SharedString::from(format!("office-type-motion:{key}")),
                    if expanded { 1_f32 } else { 0_f32 },
                    motion::reveal_transition(expanded),
                    window,
                    cx,
                );
                types = types.child(gpui_kit::base::motion::MotionReveal::new(
                    SharedString::from(format!("office-type-reveal:{key}")),
                    progress,
                    rows.into_any_element(),
                ));
            }
            let progress = gpui_kit::base::motion::transition(
                SharedString::from(format!("office-day-motion:{day}")),
                if open { 1_f32 } else { 0_f32 },
                motion::reveal_transition(open),
                window,
                cx,
            );
            body = body.child(gpui_kit::base::motion::MotionReveal::new(
                SharedString::from(format!("office-day-reveal:{day}")),
                progress,
                types.into_any_element(),
            ));
        }
        if self.office.truncated {
            body = body.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("仅展示已读取范围内最近的 100 个产物。"),
            );
        }
        body.into_any_element()
    }
}

fn file_category(path: &str) -> &'static str {
    if path.starts_with("assets/") || path.contains("/assets/") {
        return "素材";
    }
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "PDF",
        "doc" | "docx" | "md" | "txt" | "rtf" => "DOC",
        "ppt" | "pptx" => "PPT",
        "xls" | "xlsx" | "csv" => "XLS",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "mp4" | "mov" | "webm"
        | "mp3" | "wav" | "flac" | "ttf" | "otf" | "woff" | "woff2" => "素材",
        _ => "其他",
    }
}
