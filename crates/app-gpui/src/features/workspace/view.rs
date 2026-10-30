use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use agent_context::git::ChangeKind;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    browser::{Entry, EntryKind, Listing, Preview, list_directory, preview_file},
    diff::{DiffRow, Expansion, RowKind, parse_patch},
    preview::PreviewPane,
    service::{WorkspaceFile, WorkspaceRead, read_workspace},
};
use crate::ui::icons::Keyline;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTab {
    #[default]
    Changes,
    Files,
}

enum DirectoryState {
    Loading,
    Ready(Listing),
    Error(String),
}

pub struct WorkspacePanel {
    root: Option<PathBuf>,
    generation: u64,
    tab: WorkspaceTab,
    header_visible: bool,
    branch: Option<String>,
    files: Vec<WorkspaceFile>,
    patches: HashMap<String, Arc<Vec<DiffRow>>>,
    expansion: Expansion,
    repository: bool,
    limited: bool,
    loading: bool,
    error: Option<String>,
    selected: Option<String>,
    load_task: Option<Task<()>>,
    directories: BTreeMap<String, DirectoryState>,
    expanded: HashSet<String>,
    directory_tasks: HashMap<String, Task<()>>,
    preview_path: Option<String>,
    preview: Option<Result<Preview, String>>,
    preview_content: Option<Entity<PreviewPane>>,
    preview_dialog_open: bool,
    preview_request: u64,
    preview_task: Option<Task<()>>,
    tree_scroll: UniformListScrollHandle,
}

impl WorkspacePanel {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            root: None,
            generation: 0,
            tab: WorkspaceTab::Changes,
            header_visible: true,
            branch: None,
            files: Vec::new(),
            patches: HashMap::new(),
            expansion: Expansion::default(),
            repository: false,
            limited: false,
            loading: false,
            error: None,
            selected: None,
            load_task: None,
            directories: BTreeMap::new(),
            expanded: HashSet::from([String::new()]),
            directory_tasks: HashMap::new(),
            preview_path: None,
            preview: None,
            preview_content: None,
            preview_dialog_open: false,
            preview_request: 0,
            preview_task: None,
            tree_scroll: UniformListScrollHandle::new(),
        }
    }

    pub fn set_workspace(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.root = Some(root);
        self.branch = None;
        self.files.clear();
        self.patches.clear();
        self.expansion = Expansion::default();
        self.repository = false;
        self.limited = false;
        self.selected = None;
        self.reset_browser();
        self.refresh(window, cx);
    }

    pub fn tab(&self) -> WorkspaceTab {
        self.tab
    }

    /// The unified shell dock owns the frame, tabs and refresh action.
    pub fn set_header_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.header_visible = visible;
        cx.notify();
    }

    /// Shell dock tabs can select the body without recreating the browser or
    /// losing the user's expanded directories / patch cards.
    pub fn set_tab(&mut self, tab: WorkspaceTab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == WorkspaceTab::Files && self.directories.is_empty() {
            self.load_directory(String::new(), window, cx);
        }
        cx.notify();
    }

    pub fn branch(&self) -> Option<String> {
        self.branch.clone()
    }

    pub fn change_count(&self) -> usize {
        self.files.len()
    }

    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.preview_dialog_open {
            window.close_dialog(cx);
            self.preview_dialog_open = false;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        // Cancel obsolete file requests too: their replies use the same root
        // generation, so they cannot revive a stale preview after refresh.
        self.reset_browser();
        if self.tab == WorkspaceTab::Files {
            self.load_directory(String::new(), window, cx);
        }
        let work = cx.background_executor().spawn(async move {
            let result = read_workspace(&root)?;
            // Parsing is bounded but still not UI work.
            let mut patches = HashMap::new();
            if let WorkspaceRead::Repository { files, .. } = &result {
                for entry in files {
                    if entry.preview_error.is_none()
                        && !entry.change.binary
                        && !entry.change.truncated
                        && !entry.change.diff.is_empty()
                    {
                        patches.insert(
                            entry.change.path.clone(),
                            Arc::new(parse_patch(&entry.change.diff)),
                        );
                    }
                }
            }
            Ok::<_, String>((result, patches))
        });
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                // Keep the last successful list on a transient error; never
                // misreport unreadability as an empty / clean repository.
                match result {
                    Ok((
                        WorkspaceRead::Repository {
                            branch,
                            files,
                            limited,
                        },
                        patches,
                    )) => {
                        this.repository = true;
                        this.branch = branch;
                        this.files = files;
                        this.patches = patches;
                        this.limited = limited;
                        let paths = this
                            .files
                            .iter()
                            .map(|f| f.change.path.clone())
                            .collect::<Vec<_>>();
                        this.expansion.reconcile(&paths, this.patches.len());
                        if !this
                            .files
                            .iter()
                            .any(|f| Some(&f.change.path) == this.selected.as_ref())
                        {
                            this.selected = None;
                        }
                    }
                    Ok((WorkspaceRead::NotRepository, _)) => {
                        this.repository = false;
                        this.branch = None;
                        this.files.clear();
                        this.patches.clear();
                        this.limited = false;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reset_browser(&mut self) {
        self.preview_request = self.preview_request.wrapping_add(1);
        self.directory_tasks.clear();
        self.directories.clear();
        self.expanded = HashSet::from([String::new()]);
        self.preview_path = None;
        self.preview = None;
        self.preview_content = None;
        self.preview_task = None;
        self.tree_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn load_directory(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if matches!(
            self.directories.get(&path),
            Some(DirectoryState::Loading | DirectoryState::Ready(_))
        ) {
            return;
        }
        self.directories
            .insert(path.clone(), DirectoryState::Loading);
        let generation = self.generation;
        let request_path = path.clone();
        let work = cx
            .background_executor()
            .spawn(async move { list_directory(&root, &request_path) });
        let key = path.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.directories.insert(
                    path,
                    match result {
                        Ok(listing) => DirectoryState::Ready(listing),
                        Err(error) => DirectoryState::Error(error),
                    },
                );
                cx.notify();
            });
        });
        self.directory_tasks.insert(key, task);
        cx.notify();
    }

    fn open_entry(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.kind == EntryKind::Directory {
            if !self.expanded.remove(&entry.path) {
                self.expanded.insert(entry.path.clone());
                self.load_directory(entry.path, window, cx);
            }
            cx.notify();
            return;
        }
        self.preview_request = self.preview_request.wrapping_add(1);
        self.preview_path = Some(entry.path.clone());
        self.preview = None;
        self.preview_content = None;
        if entry.kind == EntryKind::Symlink {
            self.preview = Some(Err(
                "为保护工作区，不预览符号链接或经符号链接访问的文件".into()
            ));
            cx.notify();
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let generation = self.generation;
        let request = self.preview_request;
        let path = entry.path.clone();
        let work = cx
            .background_executor()
            .spawn(async move { preview_file(&root, &entry.path) });
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            let preview = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation || this.preview_request != request {
                    return;
                }
                this.preview_content = preview
                    .as_ref()
                    .ok()
                    .map(|preview| cx.new(|cx| PreviewPane::new(path, preview.clone(), false, cx)));
                this.preview = Some(preview);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_request = self.preview_request.wrapping_add(1);
        self.preview_task = None;
        self.preview_path = None;
        self.preview = None;
        self.preview_content = None;
        cx.notify();
    }

    fn expand_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ok(preview)) = &self.preview else {
            return;
        };
        let Some(path) = self.preview_path.clone() else {
            return;
        };
        let preview = preview.clone();
        let pane = cx.new(|cx| PreviewPane::new(path.clone(), preview, true, cx));
        self.preview_dialog_open = true;
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            dialog
                .entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide)
                .title(display_path(&path))
                .w(px(1100.))
                .child(div().h(px(620.)).w_full().min_w_0().child(pane.clone()))
                .on_close(move |_, _, cx| {
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |this, _| this.preview_dialog_open = false);
                    });
                })
        });
    }

    fn change_card(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.files[index];
        let file = &entry.change;
        let open = self.expansion.is_open(&file.path);
        let (name, dir) = split_path(&file.path);
        let theme = cx.theme().clone();
        let color = match file.kind {
            ChangeKind::Add => rgb(0x059669),
            ChangeKind::Delete => rgb(0xe11d48),
            ChangeKind::Update => rgb(0xd48a00),
        };
        let path = file.path.clone();
        let header = h_flex()
            .id(("workspace-change", index))
            .h(px(32.))
            .px_2()
            .gap(px(6.))
            .w_full()
            .rounded_md()
            .cursor_pointer()
            .hover(|row| row.bg(theme.muted))
            .when(self.selected.as_ref() == Some(&path), |row| {
                row.bg(theme.muted)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.expansion.toggle(&path);
                this.selected = Some(path.clone());
                cx.notify();
            }))
            .child(
                if open {
                    Keyline::ChevronDown.icon()
                } else {
                    Keyline::ChevronRight.icon()
                }
                .size(px(12.))
                .text_color(theme.muted_foreground),
            )
            .child(Keyline::File.icon().size(px(14.)).text_color(color))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(12.))
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(theme.muted_foreground)
                            .child(dir),
                    ),
            )
            .child(totals(file.adds, file.dels));
        let mut card = v_flex().w_full().gap(px(3.)).child(header);
        if open {
            if let Some(rows) = self.patches.get(&file.path) {
                let rows = rows.clone();
                let count = rows.len();
                let copy = file.diff.clone();
                card = card.child(
                    v_flex()
                        .w_full()
                        .border_1()
                        .border_color(theme.border)
                        .rounded_md()
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .h(px(29.))
                                .px_2()
                                .gap(px(6.))
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(10.5))
                                        .child(display_path(&file.path)),
                                )
                                .child(
                                    Button::new(("copy-patch", index))
                                        .ghost()
                                        .small()
                                        .label("复制")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                ),
                        )
                        .child(
                            uniform_list(("workspace-patch", index), count, move |range, _, cx| {
                                range
                                    .map(|row| render_diff_row(&rows[row], cx))
                                    .collect::<Vec<_>>()
                            })
                            .h(px((count as f32 * 21.).clamp(21., 340.)))
                            .w_full(),
                        ),
                );
            } else {
                let message = entry.preview_error.clone().unwrap_or_else(|| {
                    if file.binary {
                        "二进制文件".into()
                    } else if file.truncated {
                        "文件过大，未显示差异".into()
                    } else {
                        "没有可显示的文本差异".into()
                    }
                });
                card = card.child(note(message, cx));
            }
        }
        card.into_any_element()
    }

    fn changes_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex()
            .id("workspace-changes-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .p_2()
            .gap(px(3.));
        if let Some(error) = &self.error {
            body = body.child(note(format!("读取改动失败：{error} · 点击刷新重试"), cx));
        }
        if self.loading {
            body = body.child(note("正在读取改动…", cx));
        }
        if self.limited {
            body = body.child(note("部分差异内容或文件列表超出读取上限", cx));
        }
        if self.files.is_empty() && !self.loading && self.error.is_none() {
            body = body.child(note(
                if self.root.is_none() {
                    "打开工作区以查看改动"
                } else if !self.repository {
                    "此工作区不是 Git 仓库"
                } else if self.limited {
                    "当前读取范围内没有工作区改动"
                } else {
                    "工作区没有未提交的改动"
                },
                cx,
            ));
        }
        for index in 0..self.files.len() {
            body = body.child(self.change_card(index, cx));
        }
        body.into_any_element()
    }

    fn directory_rows(&self, path: &str, depth: usize, rows: &mut Vec<TreeRow>) {
        match self.directories.get(path) {
            None | Some(DirectoryState::Loading) => rows.push(TreeRow::Note {
                path: path.into(),
                depth,
                text: "正在加载…".into(),
                retry: false,
            }),
            Some(DirectoryState::Error(error)) => rows.push(TreeRow::Note {
                path: path.into(),
                depth,
                text: format!("读取失败：{error} · 重试"),
                retry: true,
            }),
            Some(DirectoryState::Ready(listing)) => {
                if listing.entries.is_empty() {
                    rows.push(TreeRow::Note {
                        path: path.into(),
                        depth,
                        text: "空文件夹".into(),
                        retry: false,
                    });
                }
                for entry in &listing.entries {
                    rows.push(TreeRow::Entry(entry.clone(), depth));
                    if entry.kind == EntryKind::Directory && self.expanded.contains(&entry.path) {
                        self.directory_rows(&entry.path, depth + 1, rows);
                    }
                }
                if listing.truncated {
                    rows.push(TreeRow::Note {
                        path: path.into(),
                        depth,
                        text: "目录项目过多，仅显示前 2000 项".into(),
                        retry: false,
                    });
                }
            }
        }
    }

    fn browser_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(path) = &self.preview_path {
            return self.file_preview(path, cx);
        }
        let Some(root) = self.root.as_ref() else {
            return note("打开工作区以浏览文件", cx);
        };
        let mut rows = Vec::new();
        self.directory_rows("", 0, &mut rows);
        let rows = Arc::new(rows);
        let tree = uniform_list(
            "workspace-tree",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| this.tree_row(&rows[index], index, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree_scroll)
        .flex_1()
        .min_h_0()
        .w_full()
        .into_any_element();
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy();
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .h(px(34.))
                    .px_3()
                    .gap(px(6.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Keyline::Folder.icon().size(px(14.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .child(display_path(&name)),
                    ),
            )
            .child(tree)
            .into_any_element()
    }

    fn tree_row(&self, row: &TreeRow, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match row {
            TreeRow::Entry(entry, depth) => {
                let directory = entry.kind == EntryKind::Directory;
                let expanded = self.expanded.contains(&entry.path);
                let entry = entry.clone();
                h_flex()
                    .id(("workspace-tree-row", index))
                    .h(px(28.))
                    .w_full()
                    .pl(px(8. + *depth as f32 * 14.))
                    .pr_2()
                    .gap(px(5.))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.muted))
                    .child(div().w(px(12.)).child(if directory {
                        (if expanded {
                            Keyline::ChevronDown.icon()
                        } else {
                            Keyline::ChevronRight.icon()
                        })
                        .size(px(11.))
                        .into_any_element()
                    } else {
                        div().into_any_element()
                    }))
                    .child(
                        (if directory {
                            if expanded {
                                Keyline::FolderOpen
                            } else {
                                Keyline::Folder
                            }
                        } else {
                            Keyline::File
                        })
                        .icon()
                        .size(px(13.))
                        .text_color(theme.muted_foreground),
                    )
                    .child(div().min_w_0().truncate().child(display_path(&entry.name)))
                    .when(entry.kind == EntryKind::Symlink, |row| {
                        row.child(div().text_color(theme.muted_foreground).child("↗"))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_entry(entry.clone(), window, cx)
                    }))
                    .into_any_element()
            }
            TreeRow::Note {
                path,
                depth,
                text,
                retry,
            } => {
                let path = path.clone();
                div()
                    .id(("workspace-tree-note", index))
                    .h(px(28.))
                    .w_full()
                    .pl(px(12. + *depth as f32 * 14.))
                    .pr_2()
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground)
                    .child(text.clone())
                    .when(*retry, |row| {
                        row.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.load_directory(path.clone(), window, cx)
                            }))
                    })
                    .into_any_element()
            }
        }
    }

    fn file_preview(&self, path: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let copy = match &self.preview {
            Some(Ok(Preview::Text { text, .. })) => Some(text.clone()),
            _ => None,
        };
        let mut body = v_flex().flex_1().min_h_0().w_full().child(
            h_flex()
                .h(px(34.))
                .px_2()
                .gap(px(6.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Button::new("workspace-preview-back")
                        .ghost()
                        .small()
                        .label("← 返回")
                        .on_click(cx.listener(|this, _, _, cx| this.close_preview(cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(10.5))
                        .child(display_path(path)),
                )
                .when(self.preview_content.is_some(), |header| {
                    header.child(
                        Button::new("workspace-expand-preview")
                            .ghost()
                            .small()
                            .label("展开")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.expand_preview(window, cx)),
                            ),
                    )
                })
                .when_some(copy, |header, copy| {
                    header.child(
                        Button::new("workspace-copy-file")
                            .ghost()
                            .small()
                            .label("复制")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    )
                }),
        );
        match &self.preview {
            None => body = body.child(note("正在加载文件…", cx)),
            Some(Err(error)) => body = body.child(note(format!("无法预览：{error}"), cx)),
            Some(Ok(_)) => {
                if let Some(pane) = &self.preview_content {
                    body = body.child(pane.clone());
                }
            }
        }
        body.into_any_element()
    }
}

#[derive(Clone)]
enum TreeRow {
    Entry(Entry, usize),
    Note {
        path: String,
        depth: usize,
        text: String,
        retry: bool,
    },
}

fn totals(adds: u32, dels: u32) -> AnyElement {
    h_flex()
        .gap(px(4.))
        .text_size(px(10.5))
        .font_family("monospace")
        .when(adds > 0, |row| {
            row.child(div().text_color(rgb(0x059669)).child(format!("+{adds}")))
        })
        .when(dels > 0, |row| {
            row.child(div().text_color(rgb(0xe11d48)).child(format!("-{dels}")))
        })
        .into_any_element()
}

fn render_diff_row(row: &DiffRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (background, color, sign) = match row.kind {
        RowKind::Add => (
            Some(Hsla::from(rgba(0x10b98114))),
            Hsla::from(rgb(0x059669)),
            "+",
        ),
        RowKind::Delete => (
            Some(Hsla::from(rgba(0xf43f5e14))),
            Hsla::from(rgb(0xe11d48)),
            "−",
        ),
        RowKind::Hunk => (Some(theme.muted), theme.muted_foreground, ""),
        _ => (None, theme.muted_foreground, ""),
    };
    h_flex()
        .h(px(21.))
        .w_full()
        .text_size(px(10.5))
        .font_family("monospace")
        .when_some(background, |line, bg| line.bg(bg))
        .child(
            div()
                .w(px(36.))
                .pr_1()
                .flex_shrink_0()
                .text_right()
                .text_color(theme.muted_foreground)
                .child(row.old.map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .w(px(36.))
                .pr_1()
                .flex_shrink_0()
                .text_right()
                .text_color(theme.muted_foreground)
                .child(row.new.map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .text_color(color)
                .child(sign),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when(row.kind == RowKind::Hunk, |code| {
                    code.text_color(theme.muted_foreground)
                })
                .child(display_code(&row.text)),
        )
        .into_any_element()
}

fn note(message: impl Into<String>, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_3()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(message.into())
        .into_any_element()
}

fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (display_path(name), format!("{}/", display_path(dir))),
        None => (display_path(path), String::new()),
    }
}

fn display_code(code: &str) -> String {
    code.chars()
        .flat_map(|ch| {
            if ch == '\t' {
                "    ".chars().collect::<Vec<_>>()
            } else if ch.is_control() || is_bidi(ch) {
                ch.escape_default().collect()
            } else {
                vec![ch]
            }
        })
        .collect()
}

fn is_bidi(ch: char) -> bool {
    matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn display_path(path: &str) -> String {
    let mut displayed = String::with_capacity(path.len());
    for ch in path.chars() {
        if ch.is_control() || is_bidi(ch) || ch == '\\' {
            displayed.extend(ch.escape_default());
        } else {
            displayed.push(ch);
        }
    }
    displayed
}

impl Render for WorkspacePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = match self.tab {
            WorkspaceTab::Changes => self.changes_body(cx),
            WorkspaceTab::Files => self.browser_body(cx),
        };
        v_flex()
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(crate::ui::theme::panel_background(cx))
            .text_color(theme.foreground)
            .when(self.header_visible, |panel| {
                panel.child(
                    h_flex()
                        .h(px(36.))
                        .px_2()
                        .gap(px(3.))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            Button::new("workspace-changes-tab")
                                .ghost()
                                .small()
                                .label(format!("改动 {}", self.files.len()))
                                .when(self.tab == WorkspaceTab::Changes, |button| {
                                    button.bg(theme.muted)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_tab(WorkspaceTab::Changes, window, cx)
                                })),
                        )
                        .child(
                            Button::new("workspace-files-tab")
                                .ghost()
                                .small()
                                .label("文件")
                                .when(self.tab == WorkspaceTab::Files, |button| {
                                    button.bg(theme.muted)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_tab(WorkspaceTab::Files, window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .when(self.tab == WorkspaceTab::Changes, |header| {
                            let adds = self.files.iter().map(|f| f.change.adds).sum();
                            let dels = self.files.iter().map(|f| f.change.dels).sum();
                            header.child(totals(adds, dels))
                        })
                        .child(
                            Button::new("refresh-workspace")
                                .ghost()
                                .small()
                                .label(if self.loading {
                                    "读取中…"
                                } else {
                                    "刷新"
                                })
                                .disabled(self.root.is_none() || self.loading)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                ),
                        ),
                )
            })
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{display_code, display_path, split_path};

    #[test]
    fn filenames_show_controls_and_bidi_overrides_without_rewriting_normal_text() {
        assert_eq!(display_path("src/中文.rs"), "src/中文.rs");
        assert_eq!(display_path("a\nb\tc\rd"), r"a\nb\tc\rd");
        assert_eq!(display_path("file\u{202e}txt"), r"file\u{202e}txt");
        assert_eq!(display_path(r"a\nb"), r"a\\nb");
    }

    #[test]
    fn literal_code_and_path_labels_do_not_interpret_repository_markup() {
        let code = "![image](https://invalid)<script>not executed</script>```";
        assert_eq!(display_code(code), code);
        assert_eq!(display_code("a\tb\u{202e}"), r"a    b\u{202e}");
        assert_eq!(split_path("src/lib.rs"), ("lib.rs".into(), "src/".into()));
    }
}
