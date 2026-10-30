use super::screen::TerminalScreen;

#[test]
fn terminal_titles_are_bounded_sanitized_and_separate_from_pty_replies() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed("\x1b]0;编辑\u{202e}器".as_bytes());
    assert_eq!(terminal.take_title(), None);
    terminal.feed(b"\x07");
    assert_eq!(terminal.take_title(), Some(Some("编辑器".into())));
    assert_eq!(terminal.take_title(), None);
    assert!(terminal.take_replies(8, 20).is_empty());

    let oversized = format!("\x1b]0;{}\x07", "界".repeat(100));
    terminal.feed(oversized.as_bytes());
    assert_eq!(terminal.take_title(), Some(Some("界".repeat(80))));

    terminal.feed(b"\x1b]0;first\x07\x1b]0;second\x07");
    assert_eq!(terminal.take_title(), Some(Some("second".into())));
    terminal.feed(b"\x1b]0;\x07");
    assert_eq!(terminal.take_title(), Some(None));
}

#[test]
fn light_ansi_colors_match_the_readable_palette_and_keep_osc_overrides() {
    use super::screen::TerminalPalette;
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(b"\x1b[32mg\x1b[33my");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.cells[0].foreground, 0x116329);
    assert_eq!(snapshot.cells[1].foreground, 0x4d2d00);
    terminal.feed(b"\x1b]4;2;#123456\x07\x1b[32mx");
    assert_eq!(terminal.snapshot().cells[2].foreground, 0x123456);
}

#[test]
fn light_palette_applies_to_defaults_ansi_reset_and_cursor() {
    use super::screen::TerminalPalette;
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(b"a\x1b[31mb\x1b[0mc\x1b[7md");
    let screen = terminal.snapshot();
    assert_eq!(screen.foreground, 0x303036);
    assert_eq!(screen.background, 0xffffff);
    assert_eq!(screen.cells[0].foreground, 0x303036);
    assert_ne!(screen.cells[1].foreground, 0x303036);
    assert_eq!(screen.cells[2].foreground, 0x303036);
    assert_eq!(screen.cells[3].foreground, 0xffffff);
    assert_eq!(screen.cells[3].background, 0x303036);
    assert_eq!(screen.cursor.unwrap().color, 0x303036);
}

#[test]
fn osc_overrides_queries_and_resets_share_the_active_palette() {
    use super::screen::TerminalPalette;
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(b"\x1b]10;#123456\x07\x1b]11;#abcdef\x07x\x1b]10;?\x07\x1b]11;?\x07");
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[0].foreground, 0x123456);
    assert_eq!(screen.background, 0xabcdef);
    let replies = terminal.take_replies(8, 20);
    assert!(
        replies
            .iter()
            .any(|reply| reply.contains("rgb:1212/3434/5656"))
    );
    assert!(
        replies
            .iter()
            .any(|reply| reply.contains("rgb:abab/cdcd/efef"))
    );
    terminal.set_palette(TerminalPalette::DARK);
    assert_eq!(terminal.snapshot().foreground, 0x123456);
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(b"\x1b]110\x07\x1b]111\x07\x1b]10;?\x07\x1b]11;?\x07");
    assert_eq!(terminal.snapshot().foreground, 0x303036);
    assert_eq!(terminal.snapshot().background, 0xffffff);
    let replies = terminal.take_replies(8, 20);
    assert!(
        replies
            .iter()
            .any(|reply| reply.contains("rgb:3030/3030/3636"))
    );
    assert!(
        replies
            .iter()
            .any(|reply| reply.contains("rgb:ffff/ffff/ffff"))
    );
}

#[test]
fn osc4_queries_capture_each_pair_before_later_sets_in_the_same_sequence() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(b"\x1b]4;2;?;2;#123456;2;?;1;#abcdef;1;?;2;#654321;2;?\x07\x1b]4;1;#fedcba\x07");
    assert_eq!(
        terminal.take_replies(8, 20),
        [
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
            "\x1b]4;2;rgb:1212/3434/5656\x07",
            "\x1b]4;1;rgb:abab/cdcd/efef\x07",
            "\x1b]4;2;rgb:6565/4343/2121\x07",
        ]
    );
    terminal.feed(b"\x1b[32mg\x1b[31mr");
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[0].foreground, 0x654321);
    assert_eq!(screen.cells[1].foreground, 0xfedcba);
    assert!(terminal.take_replies(8, 20).is_empty());
}

#[test]
fn queries_before_and_after_resets_in_one_feed_keep_their_original_colors() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(
        concat!(
            "\x1b]4;2;#123456;3;#abcdef\x07",
            "\x1b]10;#654321;#111111;#fedcba\x07",
            "\x1b]4;2;?\x07\x1b]104;2\x07\x1b]4;2;?;3;?\x07",
            "\x1b]104\x07\x1b]4;3;?\x07",
            "\x1b]10;?\x07\x1b]110\x07\x1b]10;?\x07",
            "\x1b]12;?\x07\x1b]112\x07\x1b]12;?\x07",
            "\x1b]11;?\x07\x1b]111\x07\x1b]11;?\x07",
        )
        .as_bytes(),
    );
    assert_eq!(
        terminal.take_replies(8, 20),
        [
            "\x1b]4;2;rgb:1212/3434/5656\x07",
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
            "\x1b]4;3;rgb:abab/cdcd/efef\x07",
            "\x1b]4;3;rgb:e5e5/c0c0/7b7b\x07",
            "\x1b]10;rgb:6565/4343/2121\x07",
            "\x1b]10;rgb:d8d8/dede/e9e9\x07",
            "\x1b]12;rgb:fefe/dcdc/baba\x07",
            "\x1b]12;rgb:d8d8/dede/e9e9\x07",
            "\x1b]11;rgb:1111/1111/1111\x07",
            "\x1b]11;rgb:1111/1515/1a1a\x07",
        ]
    );
}

#[test]
fn dynamic_queries_observe_cursor_fallback_and_earlier_fields_not_later_sequences() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(
        concat!(
            "\x1b]10;#123456;?;?\x1b\\",
            "\x1b]10;?;#abcdef;#fedcba\x07",
            "\x1b]10;#654321\x07",
            "\x1b]10;?;?;?\x1b\\",
            "\x1b]112\x07\x1b]12;?\x07\x1b]110\x07",
        )
        .as_bytes(),
    );
    assert_eq!(
        terminal.take_replies(8, 20),
        [
            "\x1b]11;rgb:1111/1515/1a1a\x1b\\",
            "\x1b]12;rgb:1212/3434/5656\x1b\\",
            "\x1b]10;rgb:1212/3434/5656\x07",
            "\x1b]10;rgb:6565/4343/2121\x1b\\",
            "\x1b]11;rgb:abab/cdcd/efef\x1b\\",
            "\x1b]12;rgb:fefe/dcdc/baba\x1b\\",
            "\x1b]12;rgb:6565/4343/2121\x07",
        ]
    );
}

#[test]
fn queued_color_replies_do_not_change_when_the_active_palette_changes() {
    use super::screen::TerminalPalette;

    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(b"\x1b]4;2;?\x07\x1b]10;?;?;?\x07");
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(
        concat!(
            "\x1b]4;2;?\x1b\\\x1b]10;?;?;?\x1b\\",
            "\x1b]4;2;#123456\x07\x1b]10;#654321\x07",
            "\x1b]4;2;?\x07\x1b]12;?\x07",
        )
        .as_bytes(),
    );
    terminal.set_palette(TerminalPalette::DARK);
    terminal.feed(
        concat!(
            "\x1b]4;2;?\x07\x1b]12;?\x07",
            "\x1b]104;2\x07\x1b]110\x07",
            "\x1b]4;2;?\x07\x1b]12;?\x07",
        )
        .as_bytes(),
    );
    terminal.set_palette(TerminalPalette::LIGHT);
    assert_eq!(
        terminal.take_replies(8, 20),
        [
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
            "\x1b]10;rgb:d8d8/dede/e9e9\x07",
            "\x1b]11;rgb:1111/1515/1a1a\x07",
            "\x1b]12;rgb:d8d8/dede/e9e9\x07",
            "\x1b]4;2;rgb:1111/6363/2929\x1b\\",
            "\x1b]10;rgb:3030/3030/3636\x1b\\",
            "\x1b]11;rgb:ffff/ffff/ffff\x1b\\",
            "\x1b]12;rgb:3030/3030/3636\x1b\\",
            "\x1b]4;2;rgb:1212/3434/5656\x07",
            "\x1b]12;rgb:6565/4343/2121\x07",
            "\x1b]4;2;rgb:1212/3434/5656\x07",
            "\x1b]12;rgb:6565/4343/2121\x07",
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
            "\x1b]12;rgb:d8d8/dede/e9e9\x07",
        ]
    );
}

#[test]
fn mixed_osc_queries_and_replies_survive_every_bel_and_st_fragment_boundary() {
    let bytes = concat!(
        "\x1b]4;2;?;2;#123456;2;?\x07",
        "\x1b[5n",
        "\x1b]104;2\x1b\\\x1b]4;2;?\x1b\\",
        "\x1b]10;#654321;?;?\x07",
        "\x1b]12;?\x1b\\",
    )
    .as_bytes();
    let expected = [
        "\x1b]4;2;rgb:9898/c3c3/7979\x07",
        "\x1b]4;2;rgb:1212/3434/5656\x07",
        "\x1b[0n",
        "\x1b]4;2;rgb:9898/c3c3/7979\x1b\\",
        "\x1b]11;rgb:1111/1515/1a1a\x07",
        "\x1b]12;rgb:6565/4343/2121\x07",
        "\x1b]12;rgb:6565/4343/2121\x1b\\",
    ];
    for split in 0..=bytes.len() {
        let mut terminal = TerminalScreen::new(12, 3);
        terminal.feed(&bytes[..split]);
        let mut replies = terminal.take_replies(8, 20);
        terminal.feed(&bytes[split..]);
        replies.extend(terminal.take_replies(8, 20));
        assert_eq!(replies, expected, "split at byte {split}");
    }
    let mut terminal = TerminalScreen::new(12, 3);
    let mut replies = Vec::new();
    for byte in bytes {
        terminal.feed(std::slice::from_ref(byte));
        replies.extend(terminal.take_replies(8, 20));
    }
    assert_eq!(replies, expected, "one byte per feed");
}

#[test]
fn unfinished_osc_does_not_reply_and_uses_the_palette_at_dispatch() {
    use super::screen::TerminalPalette;

    for terminator in ["\x07", "\x1b\\"] {
        let mut terminal = TerminalScreen::new(12, 3);
        terminal.feed(b"\x1b]4;2;?;2;#123456;2;?");
        assert!(terminal.take_replies(8, 20).is_empty());
        terminal.set_palette(TerminalPalette::LIGHT);
        for byte in terminator.as_bytes() {
            terminal.feed(std::slice::from_ref(byte));
        }
        terminal.set_palette(TerminalPalette::DARK);
        assert_eq!(
            terminal.take_replies(8, 20),
            [
                format!("\x1b]4;2;rgb:1111/6363/2929{terminator}"),
                format!("\x1b]4;2;rgb:1212/3434/5656{terminator}"),
            ]
        );
    }
}

#[test]
fn color_interception_preserves_text_csi_mouse_modes_and_interleaved_reply_order() {
    use alacritty_terminal::term::TermMode;

    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(
        concat!(
            "abc\x1b[2;3H\x1b[1;3;4;31mZ",
            "\x1b[?1000;1006h",
            "\x1b[5n\x1b]4;2;?\x07\x1b[18t",
            "\x1b]4;2;#123456;2;?\x1b\\",
            "\x1b[14t\x1b[6n",
            "\x1b]104;2\x07\x1b]4;2;?\x07",
        )
        .as_bytes(),
    );
    let screen = terminal.snapshot();
    assert_eq!(
        screen.cells[..3]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>(),
        "abc"
    );
    let z = screen.cells.iter().find(|cell| cell.text == "Z").unwrap();
    assert_eq!((z.row, z.column), (1, 2));
    assert!(z.bold && z.italic && z.underline);
    assert_eq!(z.foreground, 0xff7a85);
    let cursor = screen.cursor.unwrap();
    assert_eq!((cursor.row, cursor.column), (1, 3));
    assert!(
        terminal
            .mode()
            .contains(TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE)
    );
    assert_eq!(
        terminal.take_replies(8, 20),
        [
            "\x1b[0n",
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
            "\x1b[8;3;12t",
            "\x1b]4;2;rgb:1212/3434/5656\x1b\\",
            "\x1b[4;60;96t",
            "\x1b[2;4R",
            "\x1b]4;2;rgb:9898/c3c3/7979\x07",
        ]
    );
    terminal.feed(b"\x1b[?1000;1006l\x1b[0m\x1b[1;2HX\x1b[?25l");
    assert!(
        !terminal
            .mode()
            .intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE)
    );
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[1].text, "X");
    assert!(!screen.cells[1].bold);
    assert!(screen.cursor.is_none());
}

#[test]
fn ansi_styles_cursor_and_alternate_screen_are_projected() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed(b"\x1b[31;1mred\x1b[0m");
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[0].text, "r");
    assert!(screen.cells[0].bold);
    assert_ne!(screen.cells[0].foreground, screen.cells[3].foreground);
    assert_eq!(screen.cursor.unwrap().column, 3);
    terminal.feed(b"\x1b[?1049hother\x1b[?1049l");
    assert_eq!(terminal.snapshot().cells[0].text, "r");
}

#[test]
fn split_utf8_wide_and_combining_characters_keep_terminal_columns() {
    let mut terminal = TerminalScreen::new(12, 3);
    let bytes = "中e\u{301}".as_bytes();
    terminal.feed(&bytes[..2]);
    terminal.feed(&bytes[2..]);
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[0].text, "中");
    assert_eq!(screen.cells[0].width, 2);
    assert_eq!(screen.cells[1].width, 0);
    assert_eq!(screen.cells[2].text, "e\u{301}");
    assert_eq!(screen.cursor.unwrap().column, 3);
}

#[test]
fn selection_copies_wide_characters_once_and_resize_preserves_content() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed("中文 hi".as_bytes());
    terminal.begin_selection(0, 0, false, 1);
    terminal.extend_selection(0, 3, true);
    assert_eq!(terminal.selected_text().as_deref(), Some("中文"));
    terminal.resize(20, 5);
    assert_eq!(terminal.dimensions(), (20, 5));
    let screen = terminal.snapshot();
    assert_eq!(screen.cells[0].text, "中");
}

#[test]
fn selecting_either_half_of_a_wide_character_highlights_the_copied_glyph() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.feed("中x".as_bytes());
    for column in [0, 1] {
        terminal.begin_selection(0, column, false, 1);
        terminal.extend_selection(0, column, true);
        assert_eq!(terminal.selected_text().as_deref(), Some("中"));
        let screen = terminal.snapshot();
        assert!(
            screen.cells[0].selected,
            "leading cell, selected half {column}"
        );
        assert!(
            screen.cells[1].selected,
            "trailing cell, selected half {column}"
        );
        assert!(!screen.cells[2].selected);
    }
}

#[test]
fn cursor_fallback_tracks_osc_foreground_and_explicit_cursor_resets() {
    use super::screen::TerminalPalette;
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.set_palette(TerminalPalette::LIGHT);
    terminal.feed(b"\x1b]10;#123456\x07");
    assert_eq!(terminal.snapshot().cursor.unwrap().color, 0x123456);

    terminal.feed(b"\x1b]12;#abcdef\x07\x1b]10;#654321\x07");
    assert_eq!(terminal.snapshot().cursor.unwrap().color, 0xabcdef);
    terminal.feed(b"\x1b]112\x07");
    assert_eq!(terminal.snapshot().cursor.unwrap().color, 0x654321);

    terminal.feed(b"\x1b]12;?\x07");
    assert!(
        terminal
            .take_replies(8, 20)
            .iter()
            .any(|reply| reply.contains("rgb:6565/4343/2121"))
    );
    terminal.feed(b"\x1b]110\x07");
    assert_eq!(
        terminal.snapshot().cursor.unwrap().color,
        TerminalPalette::LIGHT.foreground
    );
}

#[test]
fn protocol_replies_keep_cursor_and_resized_character_and_pixel_sizes_in_order() {
    let mut terminal = TerminalScreen::new(12, 3);
    terminal.resize(20, 5);
    terminal.feed(b"\x1b[18t\x1b[14t\x1b[3;4H\x1b[6n");
    assert_eq!(
        terminal.take_replies(8, 20),
        ["\x1b[8;5;20t", "\x1b[4;100;160t", "\x1b[3;4R"]
    );
    assert!(terminal.take_replies(8, 20).is_empty());
}

#[test]
fn scrolled_selection_uses_history_coordinates_and_hides_the_offscreen_cursor() {
    let mut terminal = TerminalScreen::new(8, 2);
    terminal.feed(b"one\r\ntwo\r\nthree");
    terminal.scroll(1);
    assert!(terminal.snapshot().cursor.is_none());
    terminal.begin_selection(0, 0, false, 1);
    terminal.extend_selection(0, 2, true);
    assert_eq!(terminal.selected_text().as_deref(), Some("one"));
    let screen = terminal.snapshot();
    assert!(screen.cells[..3].iter().all(|cell| cell.selected));
    terminal.scroll_to_bottom();
    let screen = terminal.snapshot();
    assert!(screen.cells.iter().all(|cell| !cell.selected));
    assert_eq!(screen.cursor.unwrap().row, 1);
}
