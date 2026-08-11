# Kitty の実装確認と WezTerm への導入分類

調査対象は Kitty リポジトリの `master` commit `5734bb5a587c1add697616d32ea831ff710abd26`（2026-08-11）です。ここでいう「安全」は Kitty のテストで意味論が保護されている設計を、WezTerm の公開 API と既存テストを保ったまま再実装できる可能性を指します。Kitty は GPL-3.0 のため、ソースコードの直接移植は行わず、アルゴリズムとテスト観点だけを利用します。

## 判定一覧

| Kitty の設計 | WezTerm の現状 | 分類 | 理由 |
| --- | --- | --- | --- |
| 行マップを回転する full-width scroll | `Screen` は `VecDeque<Line>` と StableRowIndex を使用 | B（段階導入） | セルコピーを避けられるが、stable row・画像・reflow の対応が必要 |
| CSI ICH/DCH の範囲 shift + 一括 clear | `Line` のセル操作を範囲単位へ改善可能 | A（最有力） | 既存の wide-cell 境界処理を維持すれば局所変更 |
| 1行 dirty + 画面 dirty + graphics dirty | WezTerm に seqno/dirty 行は既存 | A/B | dirty 契約の明示と upload 範囲の計測は安全。描画再設計は B |
| 2048行 segment + ring history | 単一 `VecDeque` の scrollback | B | メモリ局所性は改善し得るが、resize/StableRowIndex を壊しやすい |
| 全 PTY を1つの `poll` I/O thread で処理 | pane ごとの reader/parser thread と socketpair | C（実験 backend のみ） | コピー削減の余地はあるが、portable-pty、順序、終了処理が異なる |
| CPUCell/GPUCell 固定配列、sprite index、TextCache | Cell/RenderState/Glyph cache が既存 | C（直接移植不可） | Unicode cluster、shaping、画像、WebGPU resource lifetime と結合 |

## 1. CSI の範囲編集

### Kitty の実装

- `linebuf_insert_lines`/`linebuf_delete_lines` は画面行の `line_map` と属性配列を範囲シフトし、空いた行のセルだけを clear します（[line-buf.c:421-468](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line-buf.c#L421-L468)）。`linebuf_index`/`reverse_index` もセル本体をコピーせず、論理行番号を回転します（[line-buf.c:364-395](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line-buf.c#L364-L395)）。
- ICH/DCH は `screen_insert_characters`/`screen_delete_characters` が count を一度に受け取り、CPUCell/GPUCell を範囲シフトします（[screen.c:971-987](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L971-L987)、[screen.c:2991-3006](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L2991-L3006)）。単純な `memmove` だけでなく、shift 前後に multicell 境界を nuke しています。
- CSI の公開操作は count を列幅に clamp し、clear と dirty 化を1回にまとめます（[screen.c:2964-2977](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L2964-L2977)、[screen.c:3008-3023](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L3008-L3023)）。

### WezTerm への安全な順序

1. `Line` に `shift_cells(range, count, direction)` と `clear_cells(range, attrs)` を追加し、n=1 と wide/combining/image placeholder は従来経路へ fallback する。
2. 既存の wide-cell、hyperlink、zone、Kitty placeholder の境界 invalidation を shift helper の前後で必ず実行する。
3. full-width の IL/DL だけ、行 map の回転を feature flag 付きで試す。左右マージンは当面コピー経路に残す。

### 追加テスト

Kitty は `kitty_tests/datatypes.py:255-345` で insert/delete の範囲、過大 count、部分領域を検証し、`kitty_tests/screen.py:123-134` で INT_MAX の scroll が busy-loop しないことを検証しています。WezTerm では以下を追加します。

- CSI `@`/`P`/`X`/`L`/`M`/`S`/`T` の count=0、count>列/行、INT_MAX。
- 左右/上下 margin、normal/alternate screen、wrap フラグ、SGR/hyperlink の保持。
- 2セル幅、multicell、combining cluster、Kitty placeholder が shift 境界を跨ぐケース。
- selection/StableRowIndex/dirty seqno が旧実装と一致する differential test。

この範囲に限定した一括 shift は A（最有力）ですが、行 map 全体の置換は B です。

## 2. 範囲 clear、CPU/GPU cell、TextCache

Kitty は `GPUCell`（色、sprite index、属性）と `CPUCell`（文字または TextCache index、hyperlink、multicell geometry）を固定長配列で分離します（[line.h:37-105](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line.h#L37-L105)）。範囲 clear は `memset` と cursor 属性の配列設定で行います（[line.c:833-849](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line.c#L833-L849)）。複数 codepoint は TextCache に intern し、GC 時に live cell の index を remap します（[text-cache.c:147-174](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/text-cache.c#L147-L174)、[screen.c:1032-1078](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L1032-L1078)）。

これは WezTerm の `Cell`/`ClusteredLine`/HarfBuzz/WebGPU の所有権モデルと異なるため、二重配列の直接導入は C です。安全な取り込みは、既存の line API に range fill と capacity 再利用を追加し、GPU 側は line seqno をキーにした immutable row cache として段階導入することです。sprite eviction、ligature、emoji/color font、selection/cursor overlay のテストなしに layout を変更してはいけません。

## 3. Dirty rendering と GPU upload

Kitty は screen-wide `is_dirty` に加え、各行 `LineAttrs.has_dirty_text` を保持します（[line-buf.c:47-55](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/line-buf.c#L47-L55)）。`screen_update_cell_data` は visible/history 行を走査し、dirty 行だけを shaping（`render_line`）し、成功後に clean にします。画像 graphics は非 dirty 行でも更新され、pixel scroll のときだけ history 全体を強制 render します（[screen.c:3965-4017](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/screen.c#L3965-L4017)）。ただし `update_line_data` 自体は各 render row に対して呼ばれ、GPU cell buffer も `render_lines * columns` 全体を map します。つまり Kitty から安全に取り込めるのは「shaping の dirty 行抑制」であり、GPU 転送の行単位差分化を実証しているわけではありません（[shaders.c:947-998](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/shaders.c#L947-L998)）。

WezTerm の seqno/dirty row は既に同じ方向です。まず「dirty の理由（text/cursor/selection/image/scroll）」と shaping/row upload 数を metrics 化し、no-op frame で再シェイプ 0、1行変更で再シェイプ 1、cursor 移動で旧/新行の2行という不変条件をテストします。GPU 転送を行単位に差分化する場合は Kitty の保証外なので、WebGPU buffer lifetime と full-buffer fallback を含む独自テストが必要です。graphics/image、pixel scroll、選択範囲、GPU upload 失敗時の dirty 保持も必須です。dirty 契約の明示は A、pane renderer 全体の差分化は B です。

## 4. Scrollback storage

Kitty の HistoryBuf は 2048 行単位の segment を遅延確保し、CPU/GPU cell と属性を同じ segment に置きます（[history.c:17-56](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/history.c#L17-L56)）。`start_of_data` と `count` の ring で push/pop を O(1) にし、満杯時は最古行を上書きします（[history.c:179-185](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/history.c#L179-L185)、[history.c:318-347](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/history.c#L318-L347)）。

WezTerm の single `VecDeque` を segment/map に変更すると、巨大 scrollback の realloc・cache miss を抑えられる可能性があります。ただし StableRowIndex、compression、resize/reflow、画像・hyperlink lifetime が結合しているため B です。最初は同じ `Screen` API を実装する backend として 100k 行のメモリ/scroll benchmark を追加し、結果が出た場合だけ切り替えます。wrap、truncation、reverse scroll、resize、OOM の回帰テストを必須とします。Kitty 側の参照テストは `kitty_tests/datatypes.py:769-821` と `kitty_tests/screen.py:730-766` です。

## 5. I/O と threading

Kitty は全 child PTY を `ChildMonitor` の一つの I/O thread が `poll` で監視します。PTY parser の writable buffer を直接取得して `read`→`commit` するため、reader thread から socketpair へコピーしません（[child-monitor.c:1637-1655](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/child-monitor.c#L1637-L1655)）。各 fd は parser buffer の空きと write buffer の有無で POLLIN/POLLOUT を切り替え、同じ poll で公平に処理します（[child-monitor.c:1773-1833](https://github.com/kovidgoyal/kitty/blob/5734bb5a587c1add697616d32ea831ff710abd26/kitty/child-monitor.c#L1773-L1833)）。

WezTerm は portable-pty の都合で pane ごとの read/parser thread と socketpair を使用しています。Kitty 方式はコピーと context switch を減らせる反面、Windows、parser backpressure、pane 順序、EOF/child death、lock order の意味論が異なります。Unix の実験 backend として1 paneから始め、旧 backend を fallback に残すのが限界です（分類 C）。必要なテストは partial escape boundary、`yes` 多 pane、write backpressure、EOF、child exit、input delay、parser output ordering です。

## 6. 取り込み方針

- 先に採用: range shift/clear、dirty reason metrics、huge-count clamp。既存経路との differential test を通過したものだけ有効化する。
- 条件付き採用: full-width row map、segmented scrollback、immutable row GPU cache。各々 feature flag と benchmark、StableRowIndex/画像/Unicode の回帰試験を要求する。
- 直接採用しない: Kitty の CPUCell/GPUCell レイアウト、過去の高頻度repaint/render頻度に関する設計議論、中央 I/O の全面置換。設計上の観測点だけを利用し、コードは独立実装する。
