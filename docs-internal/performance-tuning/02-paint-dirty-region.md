# 02 paint全体再構築とdirty region

## 対象と追加調査

wezterm-gui/src/termwindow/render/paint.rs:17-105,169-285 は、paintごとにquad allocationとUI itemをclearし、全pane、tab bar、split、border、modalを再構築します。quad不足/atlas不足時は同じframeを再試行します。

quad_generation、shape cache、line quad cacheとの依存があるため、dirty lineだけを再利用すると、hit-testが古い、atlas更新後にglyphが欠ける、といった不整合を作りやすいです。

## 壊してはいけない動作

- cursor、selection、hover hyperlink、blink、tab/split/modalを単独invalidateできる。
- allocated_more_quads()後の再paintを省略しない。
- UI itemの座標とmouse hit-testの座標を同じ世代で保つ。
- window background/画像/透過時のclear範囲を維持する。

## 追加テスト

- 出力、cursor blink、selection、hover、tab切替、split resize、modal表示を別々に更新する。
- 複数pane、背景画像、透過、tab barを組み合わせた手動スモーク。
- gui.paint.impl、quad.map、paint_pane.linesを全行数/dirty行数とともに記録。
- atlas拡張、shape cache clear、font reload時は全再描画になることを確認。

## 安全な実装順

まずdirty/全行の比率をmetrics化し、quad allocationの再利用だけを行います。dirty-region化は画像・modal・UI itemの依存グラフを明示してから実装します。破壊リスクは高です。
