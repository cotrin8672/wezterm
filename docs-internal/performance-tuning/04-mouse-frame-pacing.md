# 04 mousemoveとフレーム制御

## 対象と追加調査

wezterm-gui/src/termwindow/mouseevent.rs:61-245,648-813 はmousemoveごとにUI逆走査、pane選択、geometry計算、hyperlink判定、必要なら再描画を行います。Windowsは window/src/os/windows/window.rs:1637-1654,1818-1847、macOSは window/src/os/macos/window.rs:3063-3088 でtimer/promiseを生成してFPS制限します。

## 壊してはいけない動作

- click/release/drag、mouse capture、mouse reportingの最後のイベントを落とさない。
- tab bar、split、scroll thumb、pane focus follows mouseのhit-testを古い座標で処理しない。
- windowごとのmax_fps、animation、surface-loss後の再描画要求を混同しない。

## 追加テスト

- click、double click、drag selection、middle paste、wheel、mouse reporting有効時のPTYイベント。
- tab bar/split境界/scroll thumb/hover link上の高速mousemove。
- 同一座標のmousemoveをcoalesceしてもrelease順序と最終座標が一致すること。
- 入力数、処理時間、再描画要求数、frame intervalをbefore/after比較。

## 実装方針

最初はpane/UI geometry世代と座標が同じならlink判定を再実行しない局所cacheに限定します。timer共有化とイベントcoalesceは別PRにし、OS別に手動スモークします。リスクは中〜高です。
