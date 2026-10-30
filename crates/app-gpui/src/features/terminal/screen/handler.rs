//! Transparent vte 0.15.0 Handler forwarding for alacritty_terminal 0.26.0.
//!
//! Forward all 71 callbacks, including methods for which Term currently uses the
//! trait's default implementation. Recheck the complete Handler trait on upgrades:
//! omitting a default method would compile but silently change terminal behavior.
//! The only interception snapshots query colors at their exact parser position.

use alacritty_terminal::Term;
use vte::ansi::{self, Handler};

use super::{Listener, TerminalPalette, indexed_color};

pub(super) struct ScreenHandler<'a> {
    pub(super) term: &'a mut Term<Listener>,
    pub(super) listener: &'a Listener,
    pub(super) palette: TerminalPalette,
}

macro_rules! forward {
    ($(fn $method:ident($($arg:ident: $ty:ty),*);)*) => {
        $(
            fn $method(&mut self, $($arg: $ty),*) {
                Handler::$method(self.term, $($arg),*);
            }
        )*
    };
}

impl Handler for ScreenHandler<'_> {
    forward! {
        fn set_title(title: Option<String>);
        fn set_cursor_style(style: Option<ansi::CursorStyle>);
        fn set_cursor_shape(shape: ansi::CursorShape);
        fn input(c: char);
        fn goto(line: i32, col: usize);
        fn goto_line(line: i32);
        fn goto_col(col: usize);
        fn insert_blank(count: usize);
        fn move_up(rows: usize);
        fn move_down(rows: usize);
        fn identify_terminal(intermediate: Option<char>);
        fn device_status(status: usize);
        fn move_forward(cols: usize);
        fn move_backward(cols: usize);
        fn move_down_and_cr(rows: usize);
        fn move_up_and_cr(rows: usize);
        fn put_tab(count: u16);
        fn backspace();
        fn carriage_return();
        fn linefeed();
        fn bell();
        fn substitute();
        fn newline();
        fn set_horizontal_tabstop();
        fn scroll_up(rows: usize);
        fn scroll_down(rows: usize);
        fn insert_blank_lines(count: usize);
        fn delete_lines(count: usize);
        fn erase_chars(count: usize);
        fn delete_chars(count: usize);
        fn move_backward_tabs(count: u16);
        fn move_forward_tabs(count: u16);
        fn save_cursor_position();
        fn restore_cursor_position();
        fn clear_line(mode: ansi::LineClearMode);
        fn clear_screen(mode: ansi::ClearMode);
        fn clear_tabs(mode: ansi::TabulationClearMode);
        fn set_tabs(interval: u16);
        fn reset_state();
        fn reverse_index();
        fn terminal_attribute(attr: ansi::Attr);
        fn set_mode(mode: ansi::Mode);
        fn unset_mode(mode: ansi::Mode);
        fn report_mode(mode: ansi::Mode);
        fn set_private_mode(mode: ansi::PrivateMode);
        fn unset_private_mode(mode: ansi::PrivateMode);
        fn report_private_mode(mode: ansi::PrivateMode);
        fn set_scrolling_region(top: usize, bottom: Option<usize>);
        fn set_keypad_application_mode();
        fn unset_keypad_application_mode();
        fn set_active_charset(index: ansi::CharsetIndex);
        fn configure_charset(index: ansi::CharsetIndex, charset: ansi::StandardCharset);
        fn set_color(index: usize, color: ansi::Rgb);
        fn reset_color(index: usize);
        fn clipboard_store(clipboard: u8, data: &[u8]);
        fn clipboard_load(clipboard: u8, terminator: &str);
        fn decaln();
        fn push_title();
        fn pop_title();
        fn text_area_size_pixels();
        fn text_area_size_chars();
        fn set_hyperlink(hyperlink: Option<ansi::Hyperlink>);
        fn set_mouse_cursor_icon(icon: ansi::cursor_icon::CursorIcon);
        fn report_keyboard_mode();
        fn push_keyboard_mode(mode: ansi::KeyboardModes);
        fn pop_keyboard_modes(count: u16);
        fn set_keyboard_mode(mode: ansi::KeyboardModes, behavior: ansi::KeyboardModesApplyBehavior);
        fn set_modify_other_keys(mode: ansi::ModifyOtherKeys);
        fn report_modify_other_keys();
        fn set_scp(char_path: ansi::ScpCharPath, update_mode: ansi::ScpUpdateMode);
    }

    fn dynamic_color_sequence(&mut self, prefix: String, index: usize, terminator: &str) {
        let rgb = indexed_color(index, self.term.colors(), self.palette);
        self.listener.0.query_color.set(Some(ansi::Rgb {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
        }));
        // Term synchronously emits ColorRequest. Listener resolves its formatter
        // immediately into PtyWrite, alongside device-status and size requests.
        Handler::dynamic_color_sequence(self.term, prefix, index, terminator);
        self.listener.0.query_color.set(None);
    }
}
