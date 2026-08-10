# Kitty Unicode Placeholder Manual Test

This document exercises the Kitty graphics protocol Unicode placeholder path
inside Neovim. It must be opened with:

```text
mise run test-kitty-unicode-placeholder
```

The launcher forces Snacks to treat this WezTerm build as supporting Unicode
placeholders and enables inline document images only for that Neovim process.

## Expected environment

Run this command in Neovim:

```vim
:lua print(vim.inspect(Snacks.image.terminal.env()))
```

The result must include:

```lua
name = "kitty"
placeholders = true
supported = true
```

## Image A: scrolling and redraw

The screenshot below must appear inline at this location. The underlying
`U+10EEEE` cells and diacritics must not appear as text or tofu glyphs.

![WezTerm status line](../docs/screenshots/wezterm-status-powerline.png)

Move this image through the viewport with `Ctrl-E` and `Ctrl-Y`, then execute
`:redraw!`. The image must remain aligned with this paragraph and must not
leave stale fragments behind.

## Spacer for viewport movement

01. Scroll across this line.
02. Scroll across this line.
03. Scroll across this line.
04. Scroll across this line.
05. Scroll across this line.
06. Scroll across this line.
07. Scroll across this line.
08. Scroll across this line.
09. Scroll across this line.
10. Scroll across this line.
11. Scroll across this line.
12. Scroll across this line.
13. Scroll across this line.
14. Scroll across this line.
15. Scroll across this line.
16. Scroll across this line.

## Image B: split and resize

![WezTerm tab bar](../docs/screenshots/wezterm-tab-edge-styled.png)

With Image B visible:

1. Run `:split`.
2. Run `:resize 10`, followed by `:resize 20`.
3. Switch windows with `Ctrl-W w`.
4. Close one split with `:close`.

Both images must be clipped to their Neovim windows, move with their text,
and disappear completely when their placeholder cells are no longer visible.

## Pass criteria

- Both screenshots render as images rather than placeholder glyphs.
- Scrolling, `:redraw!`, split creation, resizing, and split removal leave no
  stale image fragments.
- Text, background colors, and the cursor remain visible over image cells.
- Quitting Neovim removes all images from the terminal.
