use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::browser::{Preview, RasterFormat};

/// The same read-only preview body is used in the dock and its expanded dialog.
/// No renderer ever receives a filesystem path or repository-controlled image URL.
pub(super) struct PreviewPane {
    pub original: Preview,
    pub path: String,
    large: bool,
    source: bool,
    source_view: Option<Entity<TextViewState>>,
    markdown_view: Option<Entity<TextViewState>>,
    image: Option<Arc<Image>>,
    clipped: bool,
    total_lines: usize,
    scroll: ScrollHandle,
}

impl PreviewPane {
    pub fn new(path: String, original: Preview, large: bool, cx: &mut Context<Self>) -> Self {
        let mut pane = Self {
            original,
            path,
            large,
            source: false,
            source_view: None,
            markdown_view: None,
            image: None,
            clipped: false,
            total_lines: 0,
            scroll: ScrollHandle::new(),
        };
        match &pane.original {
            Preview::Text { text, .. } => {
                let (body, clipped) = bounded_source(text, large);
                pane.clipped = clipped;
                pane.total_lines = text.split('\n').count();
                let numbered = body
                    .split('\n')
                    .enumerate()
                    .map(|(n, line)| format!("{:>4}  {}", n + 1, escape_html(line)))
                    .collect::<Vec<_>>()
                    .join("\n");
                pane.source_view =
                    Some(cx.new(|cx| TextViewState::html(&format!("<pre>{numbered}</pre>"), cx)));
                let markdown = pane.path.rsplit_once('.').is_some_and(|(_, ext)| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                });
                if markdown {
                    let end = char_boundary(text, 200_000);
                    pane.markdown_view =
                        Some(cx.new(|cx| TextViewState::markdown(&text[..end], cx)));
                }
            }
            Preview::Image { bytes, format, .. } => {
                let format = match format {
                    RasterFormat::Png => ImageFormat::Png,
                    RasterFormat::Jpeg => ImageFormat::Jpeg,
                    RasterFormat::Gif => ImageFormat::Gif,
                    RasterFormat::Webp => ImageFormat::Webp,
                };
                // Image decoding uses GPUI's image cache/background machinery;
                // only already-scoped bounded bytes cross this boundary.
                pane.image = Some(Arc::new(Image::from_bytes(format, bytes.as_ref().clone())));
            }
            _ => {}
        }
        pane
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut content = v_flex().w_full().h_full().min_h_0().min_w_0().gap(px(3.));
        if self.markdown_view.is_some() {
            content = content.child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap(px(4.))
                    .flex_shrink_0()
                    .child(
                        Button::new("markdown-rendered")
                            .ghost()
                            .small()
                            .label("预览")
                            .when(!self.source, |button| button.bg(theme.muted))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.source = false;
                                this.scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("markdown-source")
                            .ghost()
                            .small()
                            .label("源码")
                            .when(self.source, |button| button.bg(theme.muted))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.source = true;
                                this.scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })),
                    ),
            );
        }
        let size =
            match &self.original {
                Preview::Text { size, text } => {
                    let rendered = !self.source && self.markdown_view.is_some();
                    let state = if rendered {
                        self.markdown_view.as_ref()
                    } else {
                        self.source_view.as_ref()
                    };
                    let clipped = if rendered {
                        text.len() > 200_000
                    } else {
                        self.clipped
                    };
                    if clipped {
                        content = content.child(message(if self.large {
                        "为保持流畅，预览内容已截断（最多 10,000 行 / 1,000,000 字符）"
                    } else {
                        "为保持流畅，仅显示前 2,000 行 / 200,000 字符；展开预览可阅读更多内容"
                    }, cx));
                    }
                    if let Some(state) = state {
                        content = content.child(
                            div()
                                .id("workspace-content-scroll")
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .overflow_scroll()
                                .track_scroll(&self.scroll)
                                .px_3()
                                .py_2()
                                .child(
                                    TextView::new(state)
                                        .selectable(true)
                                        .stream_fade(false)
                                        // Authoritative override prevents both rendering
                                        // and intrinsic-size probes from loading local,
                                        // remote, data or HTML-embedded document images.
                                        .image_source(|_| placeholder_image())
                                        .on_link_click(|_, _, _, _| {}),
                                ),
                        );
                    }
                    *size
                }
                Preview::Image { size, .. } => {
                    if let Some(image) = &self.image {
                        content = content.child(
                            div().flex_1().min_h_0().w_full().p_3().child(
                                img(image.clone())
                                    .w_full()
                                    .h_full()
                                    .with_loading(|| {
                                        div()
                                            .text_size(px(11.))
                                            .child("正在解码图片…")
                                            .into_any_element()
                                    })
                                    .with_fallback(|| {
                                        div()
                                            .text_size(px(11.))
                                            .child("图片损坏或格式不支持，无法预览")
                                            .into_any_element()
                                    })
                                    .object_fit(ObjectFit::Contain),
                            ),
                        );
                    }
                    *size
                }
                Preview::Binary { size } => {
                    content = content.child(message("二进制或不支持的文件，无法预览。", cx));
                    *size
                }
                Preview::TooLarge { size } => {
                    content = content.child(message(
                        "文件过大，无法预览。文本上限 2 MiB，图片上限 8 MiB。",
                        cx,
                    ));
                    *size
                }
            };
        content.child(
            h_flex()
                .h(px(27.))
                .px_3()
                .gap(px(10.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(theme.border)
                .text_size(px(10.))
                .text_color(theme.muted_foreground)
                .child(if self.image.is_some() {
                    "图片"
                } else {
                    "文件"
                })
                .when(self.source_view.is_some(), |row| {
                    row.child(format!("{} 行", self.total_lines))
                })
                .child(format_size(size))
                .child(div().flex_1())
                .child("只读"),
        )
    }
}

fn message(message: &str, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(message.to_owned())
        .into_any_element()
}

fn placeholder_image() -> ImageSource {
    // A self-contained inert image; no URI reaches the image cache.
    ImageSource::Image(Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#.to_vec(),
    )))
}

fn char_boundary(text: &str, cap: usize) -> usize {
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn bounded_source(text: &str, large: bool) -> (&str, bool) {
    let cap = if large { 1_000_000 } else { 200_000 };
    let lines = if large { 10_000 } else { 2_000 };
    let mut end = char_boundary(text, cap);
    if let Some((offset, _)) = text[..end].match_indices('\n').nth(lines - 1) {
        end = offset;
    }
    (&text[..end], end < text.len())
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\t' => escaped.push_str("    "),
            ch if ch.is_control() => escaped.extend(ch.escape_default()),
            ch if matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') => {
                escaped.extend(ch.escape_default())
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn format_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.)
    } else {
        format!("{:.1} MiB", size as f64 / (1024. * 1024.))
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_source, escape_html};

    #[test]
    fn preview_caps_preserve_utf8_and_bound_both_characters_and_lines() {
        let text = "中".repeat(100_000);
        let (body, clipped) = bounded_source(&text, false);
        assert!(clipped);
        assert_eq!(body.len(), 199_998);
        let many = "line\n".repeat(3_000);
        let (body, clipped) = bounded_source(&many, false);
        assert!(clipped);
        assert_eq!(body.split('\n').count(), 2_000);
        assert_eq!(bounded_source(&many, true), (many.as_str(), false));
    }

    #[test]
    fn source_markup_cannot_inject_images_links_or_close_the_code_block() {
        assert_eq!(
            escape_html("</pre><img src='file:///secret'>&"),
            "&lt;/pre&gt;&lt;img src='file:///secret'&gt;&amp;"
        );
        assert_eq!(escape_html("\t\u{202e}"), r"    \u{202e}");
    }
}
