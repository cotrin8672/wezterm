# 05 terminal print・scrollback圧縮・属性clone

## 対象と追加調査

term/src/terminalstate/performer.rs:190-333 のflush_printはNFC判定、grapheme分割、Unicode幅、zero-width処理、pen clone、Kitty判定、セル更新を行います。term/src/screen.rs:663-710 はscrollbackへ移す行を wezterm-surface/src/line/line.rs:1188-... のcompress_for_scrollbackで圧縮します。line.rs:1268-... のchangesは属性境界ごとにChangeを作ります。

## 壊してはいけない動作

- NFC設定、grapheme境界、zero-width whitespace、double-width補助セル。
- compressed/uncompressedでCell列、attrs、wrapped、last-cell-width、hyperlink、imageが同値。
- Change列の順序、属性境界、dirty seqno。

## 追加テスト

- term/src/test/mod.rs にASCII、結合文字、emoji ZWJ、zero-width、double-width、NFC on/off。
- scrollback有効時の圧縮前後でCell/attrs/hyperlinkを比較。
- wezterm-surface/src/line/test.rs の圧縮snapshotにwrapped/double-wide/attrsを追加。
- changesを逐次更新とフル再構築で比較するプロパティテスト。

## 実装方針

最初はVec/Stringのcapacity再利用、mem::take、ASCII fast pathなど意味論を変えない変更に限定します。CPUだけでなくalloc数とscrollbackメモリも測定します。リスクは高です。
