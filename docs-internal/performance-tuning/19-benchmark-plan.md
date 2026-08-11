# 19 ベンチマークと受け入れ基準

## 現状

criterion benchはrangeset、termwiz cell、wezterm-char-props wcwidthなどに限られ、terminal input、CSI edit、hyperlink、Kitty refresh、paint frameの継続benchは不足しています。GUIにはgui.paint.impl、quad.map、shape/glyph cache、atlas関連metricsがあります。

## 追加するworkload

- terminal: yes相当のASCII、CJK/emoji、長いwrapped line、CSI 1000@/1000P/1000D。
- surface: scrollback圧縮、左右マージン、implicit hyperlink、重複rule。
- Kitty: 大scrollbackで1行placeholder、resize、screen切替、placement/delete。
- parser: OSC/APC反復、巨大payload後の小payload、chunk境界変更。
- GUI: 1/複数pane、背景画像、selection/cursor/blink、font fallback、高頻度mousemove。
- I/O: pane数、proc数、slow consumer、SSH wait値。

## 必須の比較値

CPU time、allocation回数、RSS、frame time、input p50/p99、queue wait、lock hold、GPU buffer/bind group作成数をbefore/afterで保存します。平均値だけで判断しません。

## 受け入れ基準

1. 対象workloadの改善幅と測定条件を記録する。
2. 対応する既存テストと新規回帰テストが通る。
3. terminal semanticsまたは描画順序を変える変更には差分比較/手動スモークがある。
4. メモリ保持量、cache eviction、GPU lifetime、reload反映の上限を説明する。
5. 1PR 1仮説に分割し、回帰時に個別revertできる。
