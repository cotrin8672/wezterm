# 08 hyperlink scan・logical line clone・regex

## 対象と追加調査

wezterm-gui/src/termwindow/render/pane.rs:312 からpaintごとにhyperlink適用を呼びます。wezterm-surface/src/line/line.rs:545-628 はwrapped logical lineをclone/appendし、文字列化してscanします。wezterm-surface/src/hyperlink.rs:187-215 はruleごとにcaptures_iter、matches sort、Arc<Hyperlink>生成を行います。

## 壊してはいけない動作

- physical lineの結合境界、match重複時の長いmatch優先。
- fancy-regexの捕捉/format展開、implicit/explicit hyperlinkの優先順位。
- attrsのセル範囲、hover highlight、dirty bit。

## 追加テスト

- wezterm-surface/src/line/test.rs:27 にwrapped line、Unicode grapheme、重複rule、長いmatch優先。
- wezterm-surface/src/hyperlink.rs にURL/mail、不正・長大入力、複数rule。
- 未変更Lineは再scanせず、変更Lineだけ再scanするdirty-bitテスト。
- 悪意ある長い入力でCPU上限とallocationを計測。

## 実装方針

最初はLineのtext/byte-to-cell mapとscan結果のcache化に限定します。regex engine置換やmatch順序変更は別設計にします。リスクは高です。
