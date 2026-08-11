# Kitty / Alacrittyとの実装比較

## 結論

「軽量ターミナルが採用している」ことは安全性の強い参考材料ですが、WezTermへそのまま移植できる保証ではありません。Kitty、Alacrittyに加えてGhosttyのdirty row/RenderState、page storage、batch I/Oも確認しました。特に、WezTerm固有のStableRowIndex、compressed Line、Kitty画像attachment、mux複数pane、WebGPU/GL二系統を考慮すると、次の3分類になります。

| 分類 | 対象 | 判断 |
| --- | --- | --- |
| A: 条件付きで先に取り込める | CSIの一括セル移動、範囲clear、parser buffer容量維持、metrics/dirty理由の明示 | 旧経路fallback、容量上限、回帰テストを先に追加 |
| B: パターンを取り込む | Alacrittyのcolumn damage、短時間lock/snapshot、ring buffer、bounded PTY read、Ghosttyのdirty RenderState/page batch | 効果は大きいが、WezTermの行ID/画像/mux設計に合わせた再設計が必要 |
| C: 直接移植しない | Kittyの高頻度repaintに関する歴史的議論、CPU/GPU二重cell配列、PTY全体の中央集約 | CPU/メモリ/latencyのトレードオフまたは設計依存が強い |

## WezTerm側の現状

- line単位のseqno/dirty判定とLine::changesは既にあるが、Alacrittyのline+column damage APIと同等ではない。
- full-width scrollの行移動はScreenのVecDeque/行操作、Line内のinsert/eraseはVec操作である。
- Kitty placeholderはdirty stable rowの部分refreshを既に持つ。full refreshだけが全物理行走査になる。
- 描画callback中のTerminal lock保持は残っている。
- parser/OSC/APC buffer、shape/glyph cache、PTY pipelineは独自設計であり、競合実装の単純置換ではない。
- KittyにはURL位置クエリの局所セル走査、Alacrittyにはhint検出があるが、WezTermのfancy-regex hyperlink規則と意味論は同等ではない。
- GhosttyはPage/Row dirtyとincremental RenderState、read→gather→parse batchを持つが、ページ単位の連続メモリ・世代管理とterminal単位threadに依存する。

## 優先して検証する差分

1. KittyのCSI bulk editをWezTermのLine APIへ適用できるか。
2. AlacrittyのLineDamageBounds相当を、画像/hyperlink/cluster境界を壊さずに追加できるか。
3. Alacrittyの短時間lockとbounded readを、WezTermのTerminal/mux APIに合わせて適用できるか。
4. Alacrittyのring storageを、StableRowIndexとscrollback compressionを含む形で評価する。
5. Ghosttyのincremental/full rebuild比較テストをWezTermのdirty/seqno差分テストへ落とし込む。

出典は [Kitty screen.c](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c)、[Kitty line.c](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line.c)、[Kitty child-monitor.c](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/child-monitor.c)、[Alacritty grid storage.rs](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/grid/storage.rs)、[Alacritty term/mod.rs](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/term/mod.rs)、[Alacritty event_loop.rs](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/event_loop.rs)、[Ghosttyの比較文書](24-ghostty-implementation.md) です。
