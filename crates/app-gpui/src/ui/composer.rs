use super::icons::Keyline;
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    h_flex,
};
use gpui_kit::*;

pub fn alpha(hex: u32, opacity: f32) -> Hsla {
    let mut color: Hsla = rgb(hex).into();
    color.a = opacity;
    color
}

pub fn pill(id: impl Into<ElementId>, icon: Keyline, label: &str, cx: &App) -> Button {
    let dark = cx.theme().is_dark();
    let background = alpha(if dark { 0x1e1e24 } else { 0xf4f4f5 }, 0.8);
    let hover = alpha(if dark { 0x2d2d35 } else { 0xe4e4e7 }, 0.7);
    let foreground = rgb(if dark { 0xa1a1aa } else { 0x52525b }).into();
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(hover)
                .active(hover),
        )
        .h(px(30.))
        .px(px(10.))
        .py(px(4.))
        .rounded(px(14.))
        .border_1()
        .border_color(alpha(if dark { 0x2d2d35 } else { 0xe4e4e7 }, 0.5))
        .bg(background)
        .text_size(px(12.))
        .line_height(px(20.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(if dark { 0xa1a1aa } else { 0x52525b }))
        .child(
            h_flex()
                .gap(px(6.))
                .items_center()
                .child(icon.icon().size(px(14.)).text_color(rgb(0x71717a)))
                .child(label.to_owned())
                .child(
                    Keyline::ChevronsUpDown
                        .icon()
                        .size(px(14.))
                        .text_color(rgb(if dark { 0x9a9aa4 } else { 0x71717a })),
                ),
        )
}
