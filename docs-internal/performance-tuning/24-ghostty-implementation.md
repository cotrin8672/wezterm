# Ghosttyの実装確認とWezTermへの導入分類

調査対象は Ghostty リポジトリの `main`（2026-08-11取得）です。Ghosttyは公式READMEで、terminalごとのread/write/render thread、OpenGL/Metal renderer、SIMDを利用するparserを説明しています。ただしこれはGhosttyの端末単位設計であり、WezTermのmux複数paneへそのまま移植できることを意味しません。

## 1. Pageベースの行ストレージ

Ghosttyの `terminal/page.zig` は、行とセルをページ境界に揃えた単一連続メモリへ配置します。ページには行配列・セル配列・style/hyperlink/grapheme用の専用領域があり、capacityを先に確保してresize時の再確保を減らします。行はpacked構造で、セル移動時にgrapheme mapとmanaged memoryを整合させます。

WezTermは `CellStorage` と `ClusteredLine`、scrollback compression、`StableRowIndex`を持つため、Ghosttyのページ形式やmmap allocatorの直接導入はCです。参考にできるのは、(a) 行データと補助データの寿命を同じ単位にまとめる、(b) capacityを明示して再確保を抑える、(c) managed memoryを使う行だけslow pathにする、という設計原則です。

### 安全な実験

- 既存 `Line` APIを変えず、ClusteredLineのString/cluster reserveだけを測る。
- 画像、hyperlink、combining/grapheme、resize/reflow、StableRowIndexを含む differential testを追加する。
- page allocatorやunsafe packed layoutは、RSS・断片化・OOM時の挙動を確認するまで導入しない。

判定: **B〜C（原理は参考、ストレージ置換は直接移植しない）**。

## 2. Dirty rowとincremental RenderState

Ghosttyの `Page`/`Row` はpage-level dirtyとrow-level dirtyを持ちます。page dirtyはページ内の全行を再構築する合図で、row dirtyは「画面上の見た目が変わった可能性」を示します。dirty trackingはfalse positiveを許しますがfalse negativeを許しません（`styled`/`hyperlink`など補助フラグはfalse positive可）。`RenderState.beginUpdate`/`endUpdate`の二段階APIでは、screen/terminal/page/rowのdirty、viewport pin、寸法を確認し、Fullなら全行、通常時は8行単位のdirty-mask走査でdirty rowだけを `RowBuilder` で再構築します。更新後にdirtyをclearし、`partial`/`full`をrendererへ渡します。

Ghosttyには、incremental stateとfrom-scratch rebuildを比較するテストがあり、dirty signal漏れで stale rowにならないことを検証します。これはWezTermの `seqno`/dirty rowと方向性が一致しますが、列範囲を持つdamage APIやWebGPU uploadの差分化を自動的に提供するものではありません。

### WezTermへの取り込み候補

- `dirty reason`（text/cursor/selection/image/scroll）とrow upload数をmetrics化する。
- no-op frameで再shape/rebuildが0、1行変更で1行、cursor移動で旧新行が更新される不変条件をテストする。
- 画像、selection、pixel scroll、resize、screen切替、GPU upload失敗時はFullまたは保守的な範囲へfallbackする。

判定: **A0〜B（dirty契約と比較テストは安全寄り、renderer差分化は段階導入）**。

## 3. RenderStateの再利用とlock境界

GhosttyのRenderStateは、描画用のrow dataを保持し、dirty rowだけを再構築します。packed row maskでdirty行をまとめて検査し、row arenaとcell配列はcapacityを保持して再利用します。plain-text rowではmanaged memoryがない場合にraw cell copyを高速経路にします。端末状態と描画状態を分離するため、WezTermの「Terminal lockを保持したままshape/quad cacheを作る」問題に対して、begin/endの二段階APIとsnapshot/RenderStateを設ける設計根拠になります。

ただしGhosttyもterminalごとのthread・page pin・世代番号を管理しており、WezTermのpane/mux lockを単に外せばよいわけではありません。まず immutable line snapshotを短時間lock内で取得し、seqnoとpage/line世代をcache keyにして、lock外でshapeを実行するのが安全です。

判定: **B（API再設計が必要）**。

## 4. PTY/I/O threading

Ghosttyはread threadを起動し、POSIXではnon-blocking fdとpollを使うgather stage、parse stageの二段パイプラインを持ちます。現行実装は4つの64 KiB bufferをringとして所有し、batch bufferの所有権をlock外でparseできるようにしています。満杯時はPTYへbackpressureを掛け、少量入力には低レイテンシのpoll経路を残します。Windowsではblocking readを中断するためCancelIoExを使います。公式READMEが説明するwrite thread/render threadも含め、terminal単位の複数thread設計です。

WezTermのpaneごとのread/parser threadとsocketpairコピーに対して、Ghosttyは「read→gather→parseのbounded batch、buffer所有権、idle pipeで待ちを起こす」という比較材料になります。一方、thread数を増やす設計をそのまま取り込むと、WezTermのpane数×domain×platformでRSSとcontext switchが増えるため、中央event loopへ置換する根拠にはなりません。

### 必須テスト

- `yes` flood、複数paneの公平性、partial escape boundary、同期更新、EOF/EIO、child exit。
- WindowsのCancelIoEx、UnixのHUP、idle wakeup、batch drain順序。
- parser lock wait、batch bytes、read-to-render latency、thread/RSSを計測する。

判定: **B〜C（batch/所有権の原則のみ採用、全面thread置換はしない）**。

## 5. Parserとprint batching

Ghosttyのstream parserはprint sliceをまとめて処理し、4096 codepoint scratchとSIMDによるUTF-8/control scanを使います。CSI/APCのbulk consumeと、OSCなどのheap slow pathを分けています。これはWezTermのOSC/APC `shrink_to_fit`、byteごとのRefCell borrow、graphemeごとの一時確保に対する比較材料になりますが、parser状態機械の互換性を保証するものではありません。

安全な取り込みは、既存parserの出力をgolden testで固定したうえで、scratch capacity再利用、print batching、ASCII/UTF-8 fast pathを独立にベンチすることです。

判定: **A0（scratch/計測の局所変更）〜B（parser loop再設計）**。

## 6. WezTermへの意味論上の注意

Ghosttyは高速なparserとSIMDを性能要因として掲げていますが、WezTermの `vtparse`/`wezterm-escape-parser`とは実装・状態機械が異なります。`stream.zig` は最大4096 codepointのスタックscratchへUTF-8をSIMDデコードし、printable runを `print_slice` としてまとめ、CSI/APCをbulk consumeします。Parser本体は中間文字4バイト・CSIパラメータ24個を固定配列に保持し、OSCなど可変長データだけをslow pathで確保します。Ghosttyのページ内grapheme/string allocatorは、OSC8 URIや多コードポイントgraphemeを通常セルから分離する設計で、WezTermのhyperlink/ClusteredLineを直接置換する証拠にはなりません。

参考にできるのは、hot pathでの専用scratch/capacity再利用、slow pathの明示、SIMD化前にparser意味論テストを固定する順序です。vtparseの`shrink_to_fit`除去やRefCell borrow削減は、Ghosttyの実装を移植するのではなく、既存状態機械を保った局所変更として扱います。

判定: **A0（局所計測）〜D（parser全面置換）**。

## 7. テスト・回帰設計

GhosttyのRenderStateには、dirty state、no-op update、1行変更、incrementalとfull rebuildの比較、highlight/selection更新のテストがあります。WezTerm側ではこれを次の形で再実装します。

- 端末意味論: ANSI/CSI、wide/combining、wrap/reflow、hyperlink、Kitty graphics、alternate screen。
- dirty: no-op/1行/cursor旧新/selection/image/resize/screen切替、GPU upload失敗時保持。
- storage: scrollback上限、StableRowIndex、圧縮・展開、100k行、OOM。
- I/O: batch境界、partial escape、EOF/EIO、複数pane公平性、Windows/Unix終了処理。
- differential: 旧WezTerm経路と新経路のscreen contents、attrs、seqno、画像・選択アンカーを比較。

## 参照

- [Ghostty README（thread構成・renderer・SIMD parser）](https://github.com/ghostty-org/ghostty/blob/main/README.md#competitive-performance)
- [Pageの連続メモリ・capacity・managed data](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/page.zig#L119-L204)
- [Page/Row dirty契約](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/page.zig#L1546-L1553) / [Rowのfalse-negative禁止](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/page.zig#L1816-L1819)
- [packed Rowとdirtyのfalse-negative禁止](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/page.zig#L1761-L1819)
- [RenderStateのdirty更新とrow再構築](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L327-L380)
- [dirty rowだけを再構築する経路](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L513-L583)
- [incremental/full rebuild比較テスト](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L1224-L1288)
- [no-op/1行変更のdirty stateテスト](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L1642-L1687)
- [ランダムANSIによるincremental対full比較](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L1315-L1368)
- [Ghosttyのread/gather/parse pipeline](https://github.com/ghostty-org/ghostty/blob/main/src/termio/Exec.zig#L127-L143)
- [gather/parse batchとlock外処理](https://github.com/ghostty-org/ghostty/blob/main/src/termio/Exec.zig#L1338-L1369)
- [SIMD UTF-8/print_slice/CSI bulk parser](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/stream.zig#L560-L720)
- [固定長Parser scratchとOSC slow path](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/Parser.zig#L172-L227)
- [RenderStateのbegin/end二段階更新](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/render.zig#L293-L328)
- [4×64KiB ring bufferとread backpressure](https://github.com/ghostty-org/ghostty/blob/main/src/termio/Exec.zig#L1172-L1210)
- [print sliceとSIMD/parser scratch](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/stream.zig#L253-L270)
