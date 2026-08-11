# 09 Terminal Mutexを描画中に保持する問題

## 対象と追加調査

mux/src/localpane.rs:207 と mux/src/termwiztermtab.rs:166 のterminal_with_lines_mutは、callback中もTerminal lockを保持します。GUI側 wezterm-gui/src/termwindow/render/pane.rs:571-573 はcallback内でviewport行のshape/cache/quad準備を行います。同じTerminal lockはlocalpane.rs:390のPTY action処理と入力でも使います。

## 壊してはいけない動作

- snapshot取得後のPTY更新を次のframeで必ずdirtyとして観測する。
- cursor、seqno、Kitty attachment、selectionとsnapshotの世代を一致させる。
- Terminal内部のmutable APIやhandlerを描画側へ漏らさない。
- snapshot cloneによってimplicit hyperlinkやcompressed Lineの表現を変えない。

## 追加テストと計測

- 高出力PTYと同時に入力、resize、mouse eventを発生させ、frame/input p99を測る。
- lock待ち時間、保持時間、snapshot seqno、描画開始seqnoをtracingで記録。
- 描画中にPTYが更新された場合、次frameで新しい文字が必ず表示されるテスト。

## 実装方針

まずlock hold histogramだけを追加します。支配的なら、短時間lockでimmutable Line viewを得る設計を検討します。Arc化やcloneはメモリ増加を別途測定します。リスクは高です。
