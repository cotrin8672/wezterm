# パフォーマンスチューニング候補の詳細調査と安全性レビュー

項目別の追加調査は [docs-internal/performance-tuning/README.md](performance-tuning/README.md) と、その配下の個別Markdownを参照してください。このファイルは全体方針と受け入れ基準の総合版です。

## 目的と前提

この文書は、性能改善候補を実装する前に、現状のコードから確認できる負荷要因、壊してはいけない動作、追加すべき回帰テストを整理したものです。ここでは実装変更を行わず、候補を「計測してから安全に小さく変更する」ための設計レビューとして扱います。

「速くなった」と判定するには、平均CPU時間だけでなく、次の互換性を維持する必要があります。

- VT/ANSI/Kittyプロトコルのセル内容、カーソル位置、wrap、dirty seqno
- SGR、double-width grapheme、Bidi、hyperlink、semantic zone、画像attachment
- scrollback、resize、alternate screen、左右マージン、reflow
- GPU描画結果、フレーム遅延、入力遅延、atlas/cacheの再構築
- PTYの順序、EOF/終了動作、設定reload、SSHのtimeout

既存の未コミット変更には触れていません。候補の多くは実測前の仮説なので、本文中の「確信度」はコード上のホットパスである確度であり、改善幅を保証するものではありません。

## 推奨する実装順

1. 既存metricsを保存した基準値にし、端末入力・描画・入力遅延・メモリを計測する。
2. 意味論を変えない割り当て削減（parser buffer、capacity、計測のサンプリング）から着手する。
3. terminalのbulk edit、hyperlink、Kitty、Mutex snapshotは、専用回帰テストを先に追加してから変更する。
4. WebGPU資源キャッシュやdirty-region描画は、GPU backendごとの画像比較/手動スモークを通して段階導入する。
5. PTYスレッド構成やプロセス情報キャッシュは、OS別の実測と長時間試験を行ってから検討する。

## 1. WebGPUのbind group・uniform・vertex buffer再生成

### 観測

`wezterm-gui/src/termwindow/render/draw.rs:26-143` では、フレームごとにtexture view、linear/nearest bind group、layer×3のrender passとuniform bind groupを作っています。各バッファの`webgpu_mut().recreate()`は `wezterm-gui/src/renderstate.rs:238-247` でmapped-at-creationのGPU bufferを新規作成し、旧bufferとswapします。

### 安全性と不変条件

- texture atlasが再作成された場合は、古いtexture view/bind groupを再利用してはいけない。
- uniformの`milliseconds`、projection、foreground HSB、samplerのlinear/nearest選択はフレームごとに正しく反映する必要がある。
- GPU bufferはsubmit完了前に上書きできない。リングbuffer/queue.write_bufferへ移行する場合、frame-in-flightの寿命を管理する。
- render passの統合はload/clear順序、z-index、alpha blend、空layerの扱いを変えないこと。

### 追加テスト・検証

- WebGPU/GLで、文字・背景画像・カラー絵文字・cursor・blink・透過背景を同じ入力から描画するスモークテスト。
- atlas再作成、ウィンドウresize、DPI変更、font reload後に新しいglyphが表示されること。
- wgpu capture/traceでbind group数、buffer作成数、GPU/CPU frame timeをbefore/after比較する。
- GPUが利用できない環境では、既存のGL pathと起動・テストが壊れないことを確認する。

### 判定

改善幅は大きそうですが、破壊リスクも高い（高）。まずbind groupのキャッシュだけを独立して計測し、buffer再利用とrender pass統合は別変更に分けます。

## 2. 全画面paintとquad再構築

### 観測

`wezterm-gui/src/termwindow/render/paint.rs:17-105,169-285` は、paintのたびにquad allocationをclearし、UI itemを再構築し、全pane・全表示行・tab/split/modalを再走査します。quad不足やatlas不足では同じpaintを再試行します。

### 安全性と不変条件

- dirty lineだけを再利用すると、cursor、selection、hyperlink hover、tab bar、modal、背景画像の依存関係を取りこぼす可能性がある。
- `allocated_more_quads()`が必要とする再paintを省略しない。
- UI itemの座標はmouse hit-testの入力でもあるため、描画キャッシュとhit-testキャッシュを不整合にしない。

### 追加テスト・検証

- 出力更新、cursor blink、selection、hover hyperlink、tab切替、split resize、modal表示を個別にdirty化したテストシナリオ。
- paneが複数、背景画像あり、透過あり、tab barありの組み合わせを手動スモークする。
- `gui.paint.impl`、`quad.map`、`paint_pane.lines`をdirty行数/全行数と併記して、再描画率を計測する。
- まず「quadを再利用するが画像/atlas変更時は全破棄」の保守的なキャッシュから始める。

### 判定

常時高負荷なら有力だが、動作依存が広い（高）。dirty-region化は最後に回し、先に計測用metricsを増やします。

## 3. GLのdraw call・UniformBuilder反復

`wezterm-gui/src/termwindow/render/draw.rs:237-272` は、layer×3で`frame.draw`し、各非空bufferでUniformBuilderとsampler設定を作ります。

### リスクとテスト

テクスチャ、blend、z-index、背景/前景の順序をまとめると見た目が変わる可能性があります。隣接quadをbatch化する前に、GLの既存描画をgolden screenshotまたは目視比較し、cursor/画像/半透明背景/下線を確認します。UniformBuilderの再利用は、フレーム時刻とsamplerの更新を忘れない単独変更にします。確信度は中、優先度はWebGPU後です。

## 4. mousemoveとフレーム制御の割り当て

`wezterm-gui/src/termwindow/mouseevent.rs:61-245,648-813` はmousemoveごとにUI逆走査、pane選択、geometry計算、hyperlink判定を同期実行します。Windowsの `window/src/os/windows/window.rs:1637-1654,1818-1847` とmacOSの `window/src/os/macos/window.rs:3063-3088` では、高fps時にtimer/promiseも生成します。

### 安全性

- mouse eventをcoalesceすると、クリック/drag/release、mouse reporting、capture中の最後の座標を落としてはいけない。
- `pane_focus_follows_mouse`、scroll thumb、tab bar、分割境界のhit-testは座標が変わった場合だけ省略できる。
- FPS timerの共有化では、windowごとのmax_fps、animation、surface再生成を混同しない。

### テスト

- クリック、double click、drag selection、middle paste、wheel、mouse reporting有効時のPTY出力。
- tab bar/split境界/scroll thumb/hover link上を高頻度に移動する手動テスト。
- イベント入力数、処理時間、再描画要求数、frame intervalを記録し、coalesce前後の最後の座標とrelease順序を比較する。

リスクは中〜高。まず「座標が同一ならlink判定を再実行しない」という局所キャッシュから始めます。

## 5. terminalのprint・scrollback圧縮・属性clone

### 観測

`term/src/terminalstate/performer.rs:190-333` の`flush_print`は、NFC判定、grapheme分割、Unicode幅計算、pen clone、Kitty placeholder判定、セル更新を行います。`term/src/screen.rs:663-710` は通常scroll時に行をscrollbackへ移す際、`wezterm-surface/src/line/line.rs:1188-...` の`compress_for_scrollback`を呼びます。`wezterm-surface/src/line/line.rs:1268-...` の`changes`も属性境界ごとに文字列/属性を組み立てます。

### 安全性

- Unicode graphemeの境界、zero-width whitespace、NFC設定、double-widthのplaceholder cellを変えない。
- scrollback圧縮は、圧縮前後のCell列、attrs、wrapped、last-cell-width、hyperlink、image attachmentが同値である必要がある。
- `changes()`の最適化は、変更範囲の順序と`Change::Text`/属性変更の境界を維持する必要がある。

### 追加テスト

- `term/src/test/mod.rs` にASCII、結合文字、絵文字ZWJ、zero-width、double-width、NFC on/offの入力ケース。
- `term/src/test/mod.rs` の既存scrollテストに、scrollbackを有効にした圧縮前後の属性・hyperlink比較を追加。
- `wezterm-surface/src/line/test.rs` の既存`compress_for_scrollback` snapshotを拡張し、attrs/wrapped/double-wideを比較。
- `changes()`について、同じLineを逐次更新した場合のChange列とフル再構築の結果を比較するプロパティテスト。

確信度は高ですが、端末互換性の中心部です。最初はcapacity再利用や`mem::take`など所有権を変えない変更に限定します。

## 6. CSI bulk editと左右マージンscroll

### 観測

`term/src/terminalstate/mod.rs:2206-2220,2273-2289` はDeleteCharacter/InsertCharacterを1セルずつ呼び出します。`wezterm-surface/src/line/line.rs:1018-1095` の各呼び出しは`Vec::remove/insert`、hyperlink/zone invalidation、seqno更新を伴います。`term/src/screen.rs:566-650` の左右マージンscrollも各行のセルをcloneして埋め戻します。

### 破壊しやすい動作

- ICH/DCHの右端で捨てるセル、wide cellの補助セル、wrap、hyperlink/zoneの無効化範囲。
- DECLRMMの左右外側を変更しないこと、Bidi mode、blank cell attrs、dirty seqno。
- 一括移動でin-place overlapを誤ると、コピー元を上書きする。

### 必須テスト

- CSI `n@`/`nP`/`nX`/`nD`をn=0,1,列数-1,列数,列数+1で実行し、既存のセル内容・attrs・cursor・dirty linesを比較。
- double-width、explicit hyperlink、implicit hyperlink、Kitty placeholder、semantic zoneを含む行で同じテスト。
- `term/src/test/mod.rs:568-650` の左右マージンscrollテストを拡張し、外側列が不変であることをassert。
- 大きなnのcriterion/heap計測は、正しさテストと分離する。

性能効果は明確ですが、リスクは最高クラス（非常に高）。一括処理を追加しても、既存の1セル処理を小さいnのfallbackとして残し、差分比較で移行します。

## 7. Kitty Unicode placeholderの全行refresh

### 観測

`term/src/terminalstate/kitty.rs:546-570` は`for_each_phys_line_mut`で全物理行を確認します。一方、`term/src/terminalstate/performer.rs:77-97` にはdirty stable rowの経路があり、通常編集は部分走査に抑えられています。既存の `term/src/test/image.rs` には、chunk内batch、cursor/SGRの無走査、affected row限定、scroll移動、screen切替、左右マージンの回帰テストがあります。

### 安全性

全走査を削るには、geometry変更、screen切替、画像placement/delete、resize、reflowをfull refresh条件として残す必要があります。placeholder候補bitが誤ると、画像が残る/消える/別行に付くため、単純な「常にskip」は不可です。

### 追加テスト

- 既存の`term/src/test/image.rs`を維持したまま、10,000行のscrollbackにplaceholderを1行だけ置き、affected row以外のscan countが増えないことを確認。
- resize、DPI/pixel geometry変更、main/alternate screen、placementの重なり、delete/retransmit失敗を追加。
- `placeholder_scan_count`をテスト外metricsにも出し、full refresh回数とセル数を記録する。

既存実装に既に多くの安全策があるため、索引化は中リスク。まずfull refreshの呼び出し理由をenum化し、理由ごとの計測を追加します。

## 8. hyperlink scan・logical line clone・regex

### 観測

`wezterm-gui/src/termwindow/render/pane.rs:312` からpaintごとにhyperlink適用を呼びます。`wezterm-surface/src/line/line.rs:545-628` はlogical lineをclone/appendして文字列化し、`wezterm-surface/src/hyperlink.rs:187-215` はruleごとに`captures_iter`、matchesのsort、HyperlinkのArc生成を行います。

### 安全性

- wrapped physical lineをlogical lineに結合する境界を変えない。
- ルールが重なる場合の「長いmatchを優先」という現仕様を維持する。
- fancy-regexはbacktrackingを含むため、regex crateへの置換は機能差・エラー処理を確認してから行う。
- hyperlink attrsの範囲、implicit/explicit hyperlinkの優先順位、hover highlightを壊さない。

### 追加テスト

- `wezterm-surface/src/line/test.rs:27` の既存hyperlinkテストに、wrapped line、Unicode grapheme、重複rule、長いmatch優先を追加。
- `wezterm-surface/src/hyperlink.rs:222-...` のrule単体テストに、複数rule・メール・URL・不正/長大入力を追加。
- 未変更lineは再scanしないこと、変更lineだけ再scanすることをdirty bitで確認。
- regex入力のCPU上限/キャンセル方針を決め、悪意ある長い文字列をベンチに入れる。

効果は大きそうですが、URLの境界仕様に依存するため高リスク。最初はLine内のtext/byte-to-cell mapのcache化に留めます。

## 9. Terminal Mutexを描画中に保持する問題

`mux/src/localpane.rs:207`、`mux/src/termwiztermtab.rs:166` の`terminal_with_lines_mut`は、callback実行中もTerminal lockを保持します。GUI側 `wezterm-gui/src/termwindow/render/pane.rs:571-573` はそのcallback内でviewport行のshape/cache/quad準備を行います。

### 安全性

immutable snapshotを作る場合、snapshot取得後にPTYが更新したseqnoや画像attachmentを描画が見落とさない仕組みが必要です。LineのcloneはCPU/メモリを増やすため、Arc化や軽量viewの方が望ましいですが、Terminal内部のmutable APIを外へ漏らさないことが前提です。

### 追加テスト・計測

- `yes`等の高出力と同時に入力・resize・mouse eventを発生させ、frame p99と入力p99を測る。
- lock取得待ち時間、保持時間、snapshot seqno、描画開始seqnoをtracingで記録。
- snapshot方式にした場合、描画中のPTY更新後に次のframeで必ずdirtyになるテスト。

リスクは高。まずlock hold histogramだけを追加し、実測で支配的な場合にsnapshot設計を行います。

## 10. vtparse/escape-parserの割り当てとRefCell

### 観測

`vtparse/src/lib.rs:527-645` はOSC/APC開始・clearのたびに`shrink_to_fit()`します。`wezterm-escape-parser/src/parser/mod.rs:130-190` の`parse_first`/`parse_first_as_vec`はbyte loop内でRefCell borrowとclosureを繰り返します。

### 安全性

- OSCの最大parameter数、C1/ST/BEL終端、APC payloadの所有権を変えない。
- `shrink_to_fit`を削除するとメモリ保持量が増えるため、上限（例: 前回容量が閾値を超えた場合のみ縮小）を明示する。
- parserのborrowを外す変更は、action callback中の再入やstate machineのground復帰を壊さない。

### 追加テスト

- `vtparse/src/lib.rs` の既存OSC/APCテストに、連続開始、空payload、巨大payload後の小payload、BEL/ST/CAN/SUB終端を追加。
- `parse_first`/`parse_first_as_vec`について、byte chunk境界を1バイトずつ/まとめて与えたaction列が一致するテスト。
- OSC/APC反復のallocation回数とcapacityを計測し、RSS増加を監視する。

これは比較的安全な初手（中〜低リスク）ですが、parserは全入力に通るため、必ず`cargo test -p vtparse -p wezterm-escape-parser`を先に通します。

## 11. shape/glyph/font cacheとHarfBuzz

### 観測

- `lfucache/src/lib.rs:225-270` のcache hitはLRU list、LFU RBTree、metricsを毎回更新します。
- `wezterm-gui/src/termwindow/render/mod.rs:950-982` はcluster描画ごとにshape cacheを取得します。
- `wezterm-font/src/shaper/harfbuzz.rs:346-347` はUTF-8 byte長をglyph clusterのcapacityに使います。
- `wezterm-font/src/lib.rs:181-225` はshapeごとにfallback Mutex、Vec、HashSetを操作します。

### 安全性

cacheキーにはfont、size、dpi、direction、presentation、unicode width、generationが含まれる必要があります。metricsを無効化/サンプリングする場合も、cache evictionやshape errorの挙動を変えないようにします。HarfBuzz clusterのbyte offsetとglyphのcell幅の対応は、単純なcapacity変更以外では変更しません。

### 追加テスト

- Latin、CJK、結合文字、emoji ZWJ、RTL、font fallback、color emoji、DPI変更、font reloadのshape結果をgolden化。
- cache hit/miss後のshape結果がcacheなしの結果と一致するテスト。
- cache generation変更後に古いglyph/shapeを使わないテスト。
- 多言語shapeのallocation/peak RSSをベンチし、capacity変更の前後を比較。

優先度は中。まずmetricsのサンプリング、`with_capacity`の過剰分削減、thread-local scratchなど意味論を変えない変更から進めます。

## 12. PTY、設定Mutex、/proc、SSH、color scheme

### 観測と候補

- `mux/src/lib.rs:283-345` はpaneごとにPTY読取thread、socketpair、parser threadを持ち、read dataを再コピーします。
- `mux/src/lib.rs`のparser経路では、readごとにconfiguration Mutexを取得する箇所があります。
- `procinfo/src/linux.rs:31-103` は`/proc`全PIDを列挙し、各PIDのstat/exe/cwd/cmdlineを読む場合があります。
- `mux/src/ssh.rs:522-555` はdeadlineより長い固定200ms pollを行います。
- `config/src/config.rs:1328-1395,1450-1478` はreload時にcolor scheme directoryとTOMLを同期走査します。

### 安全性

PTYの統合は、backpressure、paneごとの順序、EOF、終了イベント、OSごとのblocking readを壊しやすいため最後にします。設定snapshot化ではreload直後の反映タイミングを保ちます。/procキャッシュはPID再利用をstarttimeで区別し、権限エラーを「プロセスなし」と混同しないことが必要です。SSH pollの短縮はtimeoutとresize通知の仕様を維持します。color scheme cacheは外部ファイル変更を検知し、reloadで古いschemeを返さないことが必要です。

### 追加テスト・検証

- PTY: 複数paneの順序、巨大burst、slow consumer、EOF、非UTF-8、parser error、pane終了を統合テスト。
- config: reload中のreader、外部scheme追加/削除/変更、構文エラー、同名schemeの優先順位。
- Linux proc: 子孫プロセス、PID再利用、権限拒否、プロセス終了競合をモックまたはfixtureでテスト。
- SSH: wait=0/1ms/10ms/200ms、resize、入力到着直後、EOFをテスト。
- 実機でpane数・プロセス数・color scheme数を変え、syscall数、lock wait、入力遅延を計測する。

いずれも実測なしに変更しない（高リスク）。特にPTYスレッド統合は独立した設計変更として扱います。

## 13. spawn queueの公平性

`window/src/spawn.rs:68-99,162-190` は高優先度queueを空にするまで低優先度queueを処理せず、1回のmain-loop復帰で実行する件数も制限されています。

高優先度タスクの連続投入で低優先度タスクが飢餓になる一方、1件ずつのwake/lock往復も発生します。件数/時間budgetとwake coalescingを導入する場合、入力・paint・shutdownの順序を変えないことが条件です。queueの順序、promise完了、例外伝播をテストする決定的なexecutorテストを追加し、負荷下のspawn待ち時間をhistogram化します。優先度は中です。

## テスト追加の最小セット

最初のPRで追加するテストは、次の小さなセットで十分です。

1. `term/src/test/mod.rs`：CSI ICH/DCH、左右マージン、double-width/hyperlink/dirty seqno。
2. `term/src/test/image.rs`：既存Kitty placeholderテストに大scrollback・resize・screen切替を追加。
3. `wezterm-surface/src/line/test.rs`：圧縮前後のCell/attrs/wrapped/hyperlink同値、wrapped hyperlinkの境界。
4. `vtparse/src/lib.rs`：OSC/APC終端とchunk分割、payload容量の回帰。
5. `wezterm-escape-parser`：`parse_first`と`parse_first_as_vec`のaction列同値。
6. GUI/mux：headless unit testが難しい箇所は、metrics + 手動スモーク手順を先に追加する。

## 受け入れ基準

各チューニングPRは、最低限次を満たしてから採用します。

- 対象workloadのCPU、alloc、frame time、入力p99のbefore/afterを記録している。
- 対応する既存テストと新規回帰テストが通る。
- terminal semanticsまたは描画順序を変える変更には、差分比較または手動スモーク結果がある。
- メモリ保持量、cache eviction、外部ファイルreload、GPU resource lifetimeの上限が説明されている。
- 1PR 1仮説に分割され、回帰時に個別revertできる。
