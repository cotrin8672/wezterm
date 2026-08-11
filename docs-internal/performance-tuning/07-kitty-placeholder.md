# 07 Kitty Unicode placeholder refresh

## 対象と追加調査

term/src/terminalstate/kitty.rs:546-570 はfor_each_phys_line_mutで全物理行を確認します。一方 term/src/terminalstate/performer.rs:77-97 にはdirty stable rowの部分refreshがあります。term/src/test/image.rs にはchunk内batch、cursor/SGR無走査、affected row限定、scroll移動、screen切替、左右マージンの回帰テストが既にあります。

## 壊してはいけない動作

full refresh条件（resize/geometry変更、screen切替、placement/delete、reflow）を残します。placeholder候補bitの誤判定によるstale image、欠落、別行attachmentを許しません。

## 追加テスト

- 10,000行scrollbackでplaceholderを1行だけ変更し、非対象行のscan countが増えないこと。
- resize、pixel geometry変更、main/alternate、重複placement、delete/retransmit失敗。
- placeholder_scan_count、cell scan数、attachment update数を本番metricsにも記録。

## 実装方針

まずfull refreshの理由を分類してmetrics化し、dirty row索引の取りこぼしをテストします。既存のterm/src/test/image.rsを削除・簡略化せず拡張します。リスクは中〜高です。
