//! Tests for inline image protocol handling

use super::*;

/// A tiny but valid 11x11 PNG, base64 encoded.
/// Taken from the reproduction in <https://github.com/wezterm/wezterm/issues/6344>.
const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAsAAAALCAYAAACprHcmAAAACXBIWXMAAAGKAAABigEzlzBYAAAAOUlEQVQYlZXOwQ0AMAzCQEdi7yaT0xWAN7JuDCac2PQKYxflycOoICOKtPIuqFCg4/LzKxiz6xjyAYh9DR1sLUN1AAAAAElFTkSuQmCC";

/// Feeding a Kitty graphics escape that requests a zero-sized placement (here `r=0,h=0`)
/// must not panic the terminal.
/// Prior to the fix for <https://github.com/wezterm/wezterm/issues/6344> this divided by zero
/// while computing the per-cell pixel deltas and took down the whole pane.
#[test]
fn kitty_zero_dimension_image_does_not_panic() {
    let mut term = TestTerm::new(3, 10, 0);

    // a=T: transmit and display, t=d: data is directly embedded,
    // f=100: PNG, r=0/h=0: zero rows / zero source height.
    let seq = format!("\x1b_Gr=0,h=0,a=T,t=d,f=100;{}\x1b\\", TINY_PNG_BASE64);
    term.print(seq.as_bytes());

    // The image is refused, so the cursor never moved;
    // Printing normal text and observing it confirms we recovered rather than crashing.
    term.print(b"ok");
    assert_visible_contents(&term, file!(), line!(), &["ok", "", ""]);
}

/// A well-formed Kitty graphic with non-zero dimensions should continue to be accepted.
/// The test passes as long as processing the image does not panic and the terminal remains usable.
#[test]
fn kitty_valid_image_is_accepted() {
    let mut term = TestTerm::new(3, 10, 0);

    let seq = format!("\x1b_Ga=T,t=d,f=100;{}\x1b\\", TINY_PNG_BASE64);
    term.print(seq.as_bytes());

    // Printing normal text and observing it shifted confirms the terminal is usable.
    term.print(b"ok");
    assert_visible_contents(&term, file!(), line!(), &["  ok", "", ""]);
}

/// When the pty has no pixel size, `cell_pixel_width`/`cell_pixel_height` are zero.
/// Displaying an image sized in cells (ie: without explicit `c=`/`r=`) must not divide by zero.
/// This is a distinct crash from the zero-dimension image above and is not caught by that guard.
/// See <https://github.com/wezterm/wezterm/issues/6344>.
#[test]
fn kitty_image_with_zero_pixel_dimensions_does_not_panic() {
    let mut term = Terminal::new(
        TerminalSize {
            rows: 3,
            cols: 80,
            // No pixel size!
            pixel_width: 0,
            pixel_height: 0,
            dpi: 0,
        },
        Arc::new(TestTermConfig { scrollback: 0 }),
        "WezTerm",
        "O_o",
        Box::new(Vec::new()),
    );

    // No `c=`/`r=`, so the placement is computed from the (zero) cell pixel
    // size, exercising the divide that previously panicked.
    let seq = format!("\x1b_Ga=T,t=d,f=100;{}\x1b\\", TINY_PNG_BASE64);
    term.advance_bytes(seq.as_bytes());

    // The image is refused, so the cursor never moved;
    // Printing normal text and observing it confirms we recovered rather than crashing.
    term.advance_bytes(b"ok");
    assert_visible_contents(&term, file!(), line!(), &["ok", "", ""]);
}

#[test]
fn kitty_unicode_placeholder_attaches_image_without_replacing_text() {
    use wezterm_cell::image::ImageCellAttachmentKind;

    let mut term = TestTerm::new(3, 10, 0);
    let seq = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=2,r=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(seq.as_bytes());
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}\u{10eeee}".as_bytes());

    let lines = term.term.screen().visible_lines();
    let cell = lines[0].visible_cells().next().unwrap();
    assert!(cell.str().starts_with('\u{10eeee}'));
    let images = cell.attrs().images().expect("placeholder image attachment");
    k9::assert_equal!(images.len(), 1);
    k9::assert_equal!(
        images[0].attachment_kind(),
        ImageCellAttachmentKind::KittyUnicodePlaceholder
    );
    k9::assert_equal!(images[0].image_id(), Some(1));
    k9::assert_equal!(images[0].z_index(), -1);
    let second = lines[0].visible_cells().nth(1).unwrap();
    let second_images = second.attrs().images().unwrap();
    k9::assert_equal!(second_images[0].top_left().x.into_inner(), 0.5);

    // A normal cell write must remove only the derived placeholder overlay.
    term.print("\rX".as_bytes());
    let lines = term.term.screen().visible_lines();
    let cell = lines[0].visible_cells().next().unwrap();
    k9::assert_equal!(cell.str(), "X");
    assert!(cell.attrs().images().is_none());
    let second = lines[0].visible_cells().nth(1).unwrap();
    let second_images = second.attrs().images().unwrap();
    k9::assert_equal!(second_images[0].top_left().x.into_inner(), 0.0);
}

#[test]
fn kitty_placeholder_refresh_is_batched_per_input_chunk() {
    let mut term = TestTerm::new(3, 20, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    let before = term.term.kitty_placeholder_refresh_stats();

    // Snacks changes SGR state around placeholder runs.  All four cells are
    // written to the same line in one PTY chunk and must be reconciled once,
    // rather than once for every intervening CSI/print flush.
    let placeholder = "\u{10eeee}\u{0305}\u{0305}";
    let redraw = format!(
        "\x1b[38;5;1m{placeholder}\x1b[4:0m{placeholder}\x1b[39m\x1b[38;5;1m{placeholder}{placeholder}"
    );
    term.print(redraw);

    let after = term.term.kitty_placeholder_refresh_stats();
    k9::assert_equal!(after.0 - before.0, 1);
    k9::assert_equal!(after.1 - before.1, 4);
    k9::assert_equal!(after.2 - before.2, 4);
}

#[test]
fn kitty_placeholder_cursor_and_sgr_traffic_does_not_scan_lines() {
    let mut term = TestTerm::new(3, 20, 1000);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}".as_bytes());
    let before = term.term.kitty_placeholder_refresh_stats();

    let mut tui_traffic = String::new();
    for idx in 0..500 {
        // Cursor positioning and SGR dominate a Neovim redraw but do not
        // mutate terminal cells, so they must not invoke the scanner.
        tui_traffic.push_str("\x1b[1;1H");
        tui_traffic.push_str(if idx % 2 == 0 {
            "\x1b[38;5;2m"
        } else {
            "\x1b[39m"
        });
    }
    term.print(tui_traffic);

    k9::assert_equal!(term.term.kitty_placeholder_refresh_stats(), before);
}

#[test]
fn kitty_placeholder_noop_refresh_does_not_rebuild_attachments() {
    let mut term = TestTerm::new(2, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}".as_bytes());
    let before = term.term.kitty_placeholder_refresh_stats();

    term.term.refresh_kitty_unicode_placeholders();

    let after = term.term.kitty_placeholder_refresh_stats();
    k9::assert_equal!(after.0 - before.0, 1);
    k9::assert_equal!(after.2, before.2);
}

#[test]
fn kitty_placeholder_redraw_reuses_existing_attachment() {
    let mut term = TestTerm::new(2, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    let placeholder = "\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}";
    term.print(transmit);
    term.print(placeholder.as_bytes());
    let before = term.term.kitty_placeholder_refresh_stats();

    // Neovim can redraw the same virtual text without changing its decoded
    // image cell.  Rewriting the placeholder must not allocate and attach a
    // replacement ImageCell just because the terminal cell was written again.
    term.print(format!("\r{placeholder}").as_bytes());

    let after = term.term.kitty_placeholder_refresh_stats();
    k9::assert_equal!(after.2, before.2);
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_some());
}

#[test]
fn kitty_placeholder_overwrite_clears_line_candidate_bit() {
    let mut term = TestTerm::new(2, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}".as_bytes());
    term.print("\rX");
    let after_overwrite = term.term.kitty_placeholder_refresh_stats();

    // EL marks only the current row.  Since the row's exact placeholder bit
    // was cleared by the overwrite, the normal edit path skips its scan.
    term.print("\x1b[2K");
    let after_edit = term.term.kitty_placeholder_refresh_stats();
    k9::assert_equal!(after_edit.0, after_overwrite.0);
    k9::assert_equal!(after_edit.1, after_overwrite.1);
    k9::assert_equal!(after_edit.2, after_overwrite.2);
}

#[test]
fn kitty_placeholder_line_edit_scans_only_the_affected_row() {
    let mut term = TestTerm::new(3, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    let placeholder = "\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}";
    term.print(format!("{placeholder}\r\n{placeholder}"));
    let before = term.term.kitty_placeholder_refresh_stats();

    // EL is common in Neovim redraws.  The erase path removes the derived
    // attachment and recomputes the line candidate bit directly, so it does
    // not need to invoke the placeholder scanner at all.
    term.print("\x1b[1;1H\x1b[2K");

    let after = term.term.kitty_placeholder_refresh_stats();
    k9::assert_equal!(after, before);
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_none());
    assert!(term.term.screen().visible_lines()[1]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_some());
}

#[test]
fn kitty_placeholder_scroll_moves_attachments_without_rescanning() {
    let mut term = TestTerm::new(3, 8, 1000);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}".as_bytes());
    let before = term.term.kitty_placeholder_refresh_stats();

    let mut scrolling = String::new();
    for _ in 0..500 {
        scrolling.push_str("\x1b[1S\x1b[1T");
    }
    term.print(scrolling);

    k9::assert_equal!(term.term.kitty_placeholder_refresh_stats(), before);
}

#[test]
fn kitty_placeholder_is_reconciled_before_same_chunk_scroll() {
    let mut term = TestTerm::new(2, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);

    // The LF scrolls immediately after the placeholder was buffered in the
    // same advance_bytes call.  Reconcile must happen before the stable-row
    // mapping changes so that the image moves with the source Line.
    term.print("\x1b[2;1H\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}\n".as_bytes());

    let lines = term.term.screen().visible_lines();
    let first = lines[0].visible_cells().next().unwrap();
    assert!(first.str().starts_with('\u{10eeee}'));
    assert!(first.attrs().images().is_some());
}

#[test]
fn kitty_placeholder_refreshes_when_switching_screens() {
    let mut term = TestTerm::new(2, 8, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);

    // Flush a main-screen placeholder and switch screens in one input chunk.
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}\x1b[?1049h".as_bytes());
    // Deleting while the alternate screen is active refreshes that screen.
    // Returning to main must also refresh the newly active screen and remove
    // its stale attachment.
    term.print("\x1b_Ga=d,d=i,i=1,q=2\x1b\\\x1b[?1049l".as_bytes());

    let lines = term.term.screen().visible_lines();
    let first = lines[0].visible_cells().next().unwrap();
    assert!(first.str().starts_with('\u{10eeee}'));
    assert!(first.attrs().images().is_none());
}

#[test]
fn kitty_placeholder_partial_width_scroll_repairs_line_index() {
    let mut term = TestTerm::new(3, 6, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);

    // Enable DECLRMM, limit scrolling to columns 2..4, then put the
    // placeholder on row 2. DL on row 1 copies only a cell slice into a Line
    // that previously had no placeholder bit.
    term.print(
        "\x1b[?69h\x1b[2;4s\x1b[2;2H\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}\x1b[1;2H\x1b[1M"
            .as_bytes(),
    );
    let lines = term.term.screen().visible_lines();
    let moved = lines[0].visible_cells().nth(1).unwrap();
    assert!(moved.str().starts_with('\u{10eeee}'));
    assert!(moved.attrs().images().is_some());

    // A later lifecycle refresh relies on the repaired line index.  If the
    // partial-width copy left a false-negative bit, this stale image survives.
    term.print("\x1b_Ga=d,d=i,i=1,q=2\x1b\\".as_bytes());
    let lines = term.term.screen().visible_lines();
    let moved = lines[0].visible_cells().nth(1).unwrap();
    assert!(moved.attrs().images().is_none());
}

#[test]
fn kitty_placeholder_partial_width_linefeed_repairs_line_index() {
    let mut term = TestTerm::new(2, 6, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);

    // A linefeed at the bottom margin reaches the same partial-width slice
    // copy as CSI scrolling, but through new_line().  Both the pre-scroll
    // stable-row reconcile and the post-scroll forced index repair are needed.
    term.print("\x1b[?69h\x1b[2;4s\x1b[2;2H\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}\n".as_bytes());
    let lines = term.term.screen().visible_lines();
    let moved = lines[0].visible_cells().nth(1).unwrap();
    assert!(moved.str().starts_with('\u{10eeee}'));
    assert!(moved.attrs().images().is_some());

    term.print("\x1b_Ga=d,d=i,i=1,q=2\x1b\\".as_bytes());
    let lines = term.term.screen().visible_lines();
    let moved = lines[0].visible_cells().nth(1).unwrap();
    assert!(moved.attrs().images().is_none());
}

#[test]
fn kitty_quiet_mode_applies_to_success_and_error_responses() {
    #[derive(Clone)]
    struct CaptureWriter(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut term = Terminal::new(
        TerminalSize {
            rows: 2,
            cols: 4,
            pixel_width: 32,
            pixel_height: 32,
            dpi: 0,
        },
        Arc::new(TestTermConfig { scrollback: 0 }),
        "WezTerm",
        "O_o",
        Box::new(CaptureWriter(Arc::clone(&captured))),
    );

    term.advance_bytes(b"\x1b_Ga=p,i=7,U=2,q=0\x1b\\");
    let response = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(response.contains("i=7;EINVAL:"));

    captured.lock().unwrap().clear();
    term.advance_bytes(b"\x1b_Ga=p,i=7,U=2,q=1\x1b\\");
    assert!(!captured.lock().unwrap().is_empty());

    captured.lock().unwrap().clear();
    term.advance_bytes(b"\x1b_Ga=p,i=7,U=2,q=2\x1b\\");
    assert!(captured.lock().unwrap().is_empty());

    term.advance_bytes(b"\x1b_Ga=p,i=7,U=2,q=9\x1b\\");
    assert!(captured.lock().unwrap().is_empty());

    term.advance_bytes(b"\x1b_Ga=p,i=not-an-id,U=2,q=0\x1b\\");
    assert!(captured.lock().unwrap().is_empty());

    captured.lock().unwrap().clear();
    term.advance_bytes(b"\x1b_Ga=q,q=1;AAAA\x1b\\");
    assert!(captured.lock().unwrap().is_empty());

    term.advance_bytes(b"\x1b_Ga=q,q=0;AAAA\x1b\\");
    let response = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(response.contains(";OK"));
}

#[test]
fn kitty_virtual_delete_and_failed_retransmit_clear_stale_overlay() {
    let mut term = TestTerm::new(2, 4, 0);
    let transmit = format!(
        "\x1b_Ga=T,t=d,f=100,i=1,U=1,c=1,r=1,q=2;{}\x1b\\",
        TINY_PNG_BASE64
    );
    term.print(transmit);
    term.print("\x1b[38;5;1m\u{10eeee}\u{0305}\u{0305}".as_bytes());
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_some());

    term.print(b"\x1b_Ga=d,d=i,i=1,q=2\x1b\\");
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_none());

    term.print(b"\x1b_Ga=p,i=1,U=1,c=1,r=1,q=2\x1b\\");
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_some());

    // Retransmission retires the old image and every placement before
    // decoding. A failed replacement must not resurrect the stale Arc.
    term.print(b"\x1b_Ga=t,t=d,f=100,i=1,q=2;%%%\x1b\\");
    assert!(term.term.screen().visible_lines()[0]
        .visible_cells()
        .next()
        .unwrap()
        .attrs()
        .images()
        .is_none());
}
