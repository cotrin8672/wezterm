# 12 shape cacheのhit更新コスト

## 対象と追加調査

lfucache/src/lib.rs:225-270 のgetは、hitごとにLRU listをremove/push、LFU RBTreeをremove/insert、metrics histogramを更新します。wezterm-gui/src/termwindow/render/mod.rs:950-982 はcluster描画ごとにshape cacheを取得します。

## 壊してはいけない動作

- cache keyのfont、size、dpi、direction、presentation、unicode width、generationを欠落させない。
- eviction順序とshape errorの扱いを変えない。
- cache hitで返すRcの寿命と内部mutable状態を壊さない。

## 追加テスト

- cacheあり/なしでLatin、CJK、emoji、RTL、fallbackのshape結果を比較。
- generation変更、font reload、config変更後に古いshapeを返さない。
- hit率、eviction数、RBTree操作時間、frame timeを記録。

## 実装方針

まずmetrics記録をサンプリングし、次に頻度更新をepoch化/簡略化する案を比較します。HashMapへの単純置換はevictionと再現性を変えるため最後です。リスクは中〜高です。
