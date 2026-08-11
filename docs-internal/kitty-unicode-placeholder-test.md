vim.g.loaded_netrw = 1

The image below must be visible directly under this title as soon as the test
opens. If only the Markdown alt text is visible, the test has failed.

![WezTerm status line](../docs/screenshots/wezterm-status-powerline.png)

This document exercises the Kitty graphics protocol Unicode placeholder path
inside Neovim. It must be opened with:

```text
mise run test-kitty-unicode-placeholder
```

The launcher forces Snacks to treat this WezTerm build as supporting Unicode
placeholders and enables inline document images only for that Neovim process.
It also requires ImageMagick's `magick.exe`, which Snacks uses to inspect the
PNG dimensions. Install it with `scoop install imagemagick` if the launcher
reports that it is missing.

To validate the launcher and Snacks environment without opening the UI, run:

```text
mise run test-kitty-unicode-placeholder -- --check
```

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

Use the screenshot directly below the document title for this test. Its
underlying `U+10EEEE` cells and diacritics must not appear as text or tofu
glyphs.

Move this image through the viewport with `Ctrl-E` and `Ctrl-Y`, then execute
`:redraw!`. The image must remain aligned with this paragraph and must not
leave stale fragments behind.

## Spacer for viewport movement

1.  Scroll across this line.
2.  Scroll across this line.
3.  Scroll across this line.
4.  Scroll across this line.
5.  Scroll across this line.
6.  Scroll across this line.
7.  Scroll across this line.
8.  Scroll across this line.
9.  Scroll across this line.
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

If an image disappears after scrolling away and back, inspect the Neovim
`snacks.nvim` checkout.  The placement delete request must use the placement
ID (`p=<placement_id>`), not the loop index used to remove it from the Lua
table.  An `ENOENT` for `p=1` while the placement IDs start at 11 is a clear
sign that the installed plugin still has this bug; update Snacks or apply the
local compatibility patch in `lua/snacks/image/image.lua` from this checkout.
When using a remote mux, also retry the image-cell lookup if the representative
cell has scrolled away between `GetLines` and `GetImageCell`.

## Pass criteria

- Both screenshots render as images rather than placeholder glyphs.
- Scrolling, `:redraw!`, split creation, resizing, and split removal leave no
  stale image fragments.
- Text, background colors, and the cursor remain visible over image cells.
- Quitting Neovim removes all images from the terminal.
