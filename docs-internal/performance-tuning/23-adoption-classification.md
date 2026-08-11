# Kitty / Alacritty 由来の取り込み候補の分類

## 結論

軽量ターミナルで長く使われている実装は有力な設計根拠ですが、WezTermへ移植できることや、回帰が起きないことを保証するものではありません。特に WezTerm 固有の `StableRowIndex`、圧縮済み scrollback、Kitty画像 attachment、複数 pane/mux、WebGPU と GL の二系統を境界条件として扱います。

ここでの「A」は、意味論を維持する fallback と差分テストを用意した場合に限る条件付き評価です。

## 19候補の分類

| 候補 | 他実装で確認できた対応 | WezTermの現状 | 分類 | 取り込み条件 |
|---|---|---|---|---|
| 01 WebGPU resource再生成 | AlacrittyのGL rendererはbuffer/atlasを長寿命化（WebGPUではない） | drawごとにview/bind group/bufferを再生成 | B | GLの寿命管理だけを参考にし、WebGPUのbind group/map同期はGPU captureで独自検証 |
| 02 dirty-region paint | Alacrittyに行・列damageとFull fallback（compositor矩形を絞る層もある） | seqno/dirty行はあるが列粒度は限定的 | B | wide/画像/hyperlink/scrollbackはfull-lineへfallback、resetを1回に限定。列damageだけではCPU shaping/quad生成は自動削減されない |
| 03 GL draw batching | Alacrittyはquad batchと既存bufferを再利用 | layerごとのdraw | B | blend/texture順序を変えないGPU回帰とフレーム計測。WebGPUへ直接移植しない |
| 04 mouse/frame pacing | Kittyは高頻度renderのCPUとのトレードオフを明記 | OS timer/promiseをフレームごとに生成する経路あり | C | render threadだけは移植せず、共有timer化を独自検証 |
| 05 print/scrollback圧縮 | Kittyに物理行map、Alacrittyにring storage | VecDeque移動と行圧縮 | B | StableRowIndex、reflow、画像・選択アンカーの差分テスト |
| 06 CSI bulk insert/delete | Kittyがcount単位の範囲shift | Lineのremove/insertを反復 | A（条件付き） | n=1/特殊セルは旧経路、wide/hyperlink/zone/placeholderを一括無効化。巨大countでbusy-loopしないテスト |
| 07 Kitty placeholder更新 | Kittyもline/image placeholderのdirty管理 | dirty stable rowと全走査refreshが併存 | B | 影響行索引を導入し、resize/screen切替/画像操作は全走査へfallback |
| 08 hyperlink scan | 直接同等実装は確認できない | logical line再構築・regex走査 | D | 競合実績を根拠にせず、既存URL/優先順位テストと計測を先行 |
| 09 Terminal lock/snapshot | AlacrittyはRenderableCellを収集後にGL描画 | shaping/quad準備をTerminal lock中に実行 | B | immutable snapshotを短時間lock内で作り、seqno整合性・PTY進行を検証 |
| 10 vtparse buffer容量 | Kittyは固定parser buffer、Alacrittyはread buffer（同じOSC戦略ではない） | OSC/APCごとに`shrink_to_fit` | A0（局所・計測依存） | 容量保持と上限超過時縮小を分離し、pane数を含むRSS上限・allocator回数を測る |
| 11 escape-parser borrow | 直接同等実装は確認できない | byteごとのRefCell borrow | A0（局所） | parse結果と再入性を維持するベンチ/プロパティテスト |
| 12 shape cache | Kittyにtext cache/sprite index、Alacrittyにglyph raster cache | LFU/RBTreeとglyph cache | C | データ構造は移植せず、cache hit率・eviction・atlas再生成を計測 |
| 13 font shaping | Kittyは再利用scratch/HarfBuzz、Alacrittyのglyph cacheはshape cacheとは別 | HarfBuzz/fallback固有 | D | scratch再利用だけを参考にし、多言語・絵文字・ligatureの形状回帰を先に追加 |
| 14 PTY pipeline | Kittyはcentral child monitor、Alacrittyはbounded read/lock tenure | paneごとread/parser/socketpair | B〜C | Unix実験backendまたはread batch上限から開始。Windows、EOF、fairness、backpressureを別検証。Kittyのwrite-buffer上限は子プロセスへの書込み側であり、PTY read backpressureと同一視しない |
| 15 config reload | 直接同等実装は確認できない | schemeを同期全走査 | D | metadata差分キャッシュを独自に設計し、設定結果同一性を確認 |
| 16 proc-info | 直接同等実装は確認できない | `/proc` 全PID走査 | D | TTL/対象PID化を独自計測。競合実装を安全性根拠にしない |
| 17 SSH poll | 直接同等実装は確認できない | 200ms固定poll | D | deadline上限修正と入力レイテンシ回帰を追加 |
| 18 spawn queue | 直接同等実装は確認できない | high優先queue連続処理 | D | fairness/wakeup coalescingの負荷試験を先行 |
| 19 benchmark導線 | Alacrittyはvtebench・reference test、KittyはCSI/History/dirtyテスト | parser/render専用benchmarkが不足 | A0（計測） | 外部コードをコピーせず、同等の差分・負荷シナリオをWezTermに追加 |

## Ghosttyで確認できた追加根拠

| Ghosttyの実装 | WezTerm未採用部分 | 分類 | 安全条件 |
|---|---|---|---|
| Page/Rowのdirtyとincremental `RenderState` | dirty rowを再構築するrenderer契約が明示されていない | A0〜B | no-op/1行/cursor/selection/image/resizeのdirty不変条件、incremental対full比較 |
| ページ単位の連続メモリ、capacity、専用grapheme/string領域 | ClusteredLineの容量再利用は限定的 | B〜C | 既存Line APIを保ち、RSS/OOM/Unicode/hyperlink/reflowを差分検証 |
| RenderStateがrow dataを保持し、dirty rowだけ再構築 | Terminal lock中にshape/quad cacheを生成 | B | immutable snapshotを短時間lock内で取得し、seqno/世代をcache keyにする |
| `beginUpdate`/`endUpdate`の二段階render契約 | lock境界とdirty resetがrender callbackに混在 | B | dirty resetをlock内、shape/quad/GPU処理をlock外に分離し、stale世代を拒否 |
| read→gather→parseのbatch pipelineとbuffer所有権 | paneごとのPTY read/parser/socketpair | B〜C | batch上限・公平性・EOF/EIO・Windows中断を維持し、全面thread置換はしない |
| terminalごとのread/write/render thread、SIMD parser | WezTermが同じthread粒度・parserを未採用 | C | thread数×paneのRSS/context switchを測り、SIMDは意味論テスト後に独自実装 |
| print slice、4096 codepoint scratch、CSI/APC bulk consume | byte/graphemeごとの処理と容量再確保候補 | A0〜B | golden parser testを固定し、scratch/ASCII fast pathだけを独立ベンチする |
| dirty漏れをincremental/full rebuild比較で検出するテスト | parser/render専用differential testが不足 | A0（計測・テスト） | 旧経路と新経路のcontents/attrs/seqno/画像/選択アンカー比較 |

## 条件付きAの実装順

1. `vtparse` の容量保持、parser borrowの局所最適化、dirty理由のメトリクス。挙動を変えずに計測できる。
2. CSI bulk shift。既存経路を残し、通常セル・full-width・n=1から段階導入する。
3. 範囲clear/capacity再利用。`Line::fill_range` が現状セル単位処理であることを踏まえ、範囲API追加と容量予約を分ける。固定容量化はpane数によるRSS増加を必ず測る。
4. その後にcolumn damage、snapshot、PTY batchを一件ずつ実験する。

## 必須の安全網

- ANSI/CSI: ICH/DCH/IL/DL、巨大count、左右マージン、wrap、wide/combining、属性、hyperlink、zone。
- Kitty graphics: placeholder、画像移動・retire、scrollback、resize、alternate screen、screen切替。
- damage/render: dirty範囲のleft/right境界、cursor旧新位置、selection、画像、pixel scroll、Full fallback、GPU upload失敗時のdirty保持。
- scrollback/storage: visible/history境界、StableRowIndex、reflow、truncate、選択アンカー、100k行負荷。
- I/O/lock: `yes` flood中の入力p99、partial escape、同期更新、EOF/EIO、child exit、複数pane公平性、PTY write backpressure。
- すべての変更は旧経路との differential test、feature flagまたは即時revert可能な単位、CPUだけでなくRSS/GPU frame time/allocator回数で判定する。
- KittyのURL位置クエリを参考にする場合も、wrapped line、Unicode幅、URL境界、hyperlink rule優先順位の差分テストを追加する。

## 参照した実装

- [Kitty screen.c / line.c / history.c / child-monitor.c（commit 5734bb5a）](https://github.com/kovidgoyal/kitty/tree/5734bb5a587c1add697616d32ea831ff710abd26)
- [Kittyのdirty rendering](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L3965-L4017)
- [Kittyの物理行map](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line-buf.c#L364-L468)
- [Alacrittyのdamage API（commit 1b2b36a6）](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/term/mod.rs#L137-L216)
- [Alacrittyのdisplay offset変更時Full damage](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/term/mod.rs#L389-L407)
- [Alacrittyのring storage](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/grid/storage.rs#L9-L70)
- [AlacrittyのPTY read batch](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/event_loop.rs#L104-L171)
- [Kitty issue #115（render頻度とCPUのトレードオフ）](https://github.com/kovidgoyal/kitty/issues/115)
