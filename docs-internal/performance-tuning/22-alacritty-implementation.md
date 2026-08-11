# Alacrittyの実装確認

調査対象は Alacritty commit [`1b2b36a6`](https://github.com/alacritty/alacritty/tree/1b2b36a64e88068ad02c95fad00ee2fad31c00bf) です。ここではアルゴリズムを比較し、WezTermの意味論を保った再実装条件を記録します。ソースコードの直接移植はしません。

## 1. 行・列 damage

`Term` は各行の `LineDamageBounds { line, left, right }` と `TermDamage::{Full, Partial}` を保持します。カーソル旧位置/新位置、erase/delete、scroll、display offsetなどを更新し、scrollback表示やresizeでは `Full` に戻します。`right` は包含境界なので、未変更状態（left=num_cols, right=0）とwide glyphの隣接セルを含めた overdamage が重要です。

GUI側にも複数のdamage矩形をマージする層があり、GL描画用のdamageとterminalのdamageを分離しています。WezTermのseqno/`Line::changes` は近い目的ですが、列範囲APIではありません。

なお、column damageを追加しただけでCPU側のshapingやquad生成が自動的に減るわけではありません。Alacrittyでも、terminal damage、renderable cell収集、compositorのswap damageは別層です。WezTermではdirty rangeをline/shape/quad cacheの再利用条件まで接続して初めてCPU効果を測れます。

### WezTermへの安全な段階導入

- まず行単位の `left/right` を任意値として追加し、画像、wide/combining cluster、hyperlink、reflow、display offset、resizeはfull-line/full-screenへfallbackする。
- damage resetはframe内で一度だけ行い、GPU upload失敗時はdirtyを保持する。
- cursor/selection/hint/overlayはtext damageと別理由として扱う。

追加テストは、dirty範囲の境界・マージ、cursorの旧新行、wide glyph overdamage、scrollback offset時のFull、alternate screen、resize、insert modeです。Alacritty側の `damage_public_usage`、cursor movement、Full damage、GUI矩形境界テストを設計の参考にします。

判定: **B（有力だが、Full fallbackと境界テストが必須）**。

## 2. ring bufferとscrollback初期化

`Storage<T>` はzero offsetを回すring bufferで、全行をrotateせず論理indexだけを更新します。visible行だけを先に初期化し、historyは必要時に確保します。grow/shrink、zero位置をまたぐrotate、hidden行のtruncateまで単体テストがあります。ただし `Grid` は常にring rotateするわけではなく、historyの有無、region開始位置、scrollback上限に応じてswap/reset経路へ切り替えます。

WezTermはすでに `VecDeque<Line>` を使っていますが、scrollでremove/insertするため、ring化は「未導入」ではなく、行移動をさらに減らす最適化候補です。`StableRowIndex`、scrollback compression、reflow、Kitty画像・hyperlink・選択アンカーが物理行と結合しているため、直接置換は危険です。

安全な実験は、まずalternate screen/full-width scrollかつscrollback無効の経路だけを対象に、旧VecDeque backendと新ring backendを同じ `Screen` APIの下で比較することです。historyあり、subregion、display offset、selection、画像attachmentは旧経路へfallbackします。ランダムなscroll/resize/CSI、visible/history境界、stable row、画像・選択アンカー、100k行、履歴上限超過のdifferential testを追加します。unsafeな行swapは初期導入では避けます。

判定: **B（設計パターンのみ採用）**。

## 3. Terminal lockとPTY readのbounded処理

Alacrittyのevent loopは `FairMutex`、1 MiB read buffer、`MAX_LOCKED_READ=65535`、`try_lock`を使い、一定量をparseしたらlockを解放します。これは高出力時にparserがlockを独占しないための公平性制御です。partial write、WouldBlock、EOF/EIO、同期更新のtimeoutもpollerで扱います。

描画側はTerminal lock中に `RenderableCell` のowned `Vec`を収集し、damageをresetしてからlockを解放し、その後GL描画・shape・batchを実行します。したがって「完全なzero-copy immutable view」ではなく、lock外で使うためのスナップショットです。公式changelogの説明だけでなく、`display/mod.rs`の実装を根拠にします。

WezTermのrender callbackは現在Terminal lock中にshape/HarfBuzz/quad cache準備を行うため、まずimmutable line snapshotを短時間lock内で取得し、seqnoとcache keyを保持したままlock外で描画するのが安全です。snapshot作成コストとstale frameの扱いを計測します。

### 必須テスト

- `yes`/1 MiB burst中の入力p99とparser lock wait。
- partial escape boundary、synchronized update timeout、EOF/EIO、child exit、partial write。
- 遅いrendererを挿入してもPTY parserが進むこと、cursor/selection/hyperlinkがstaleにならないこと。

判定: **B（短時間lockとbounded readの考え方を採用し、APIは再設計）**。

## 4. Renderable contentとGPU buffer再利用

`display/content.rs` は表示対象のiteratorを作り、空cellやwide spacerをrendererへ渡さない構造です。AlacrittyのOpenGL rendererはVAO/EBO/VBOとbatch領域を初期化時に確保し、frameごとは既存bufferへデータをuploadしてinstanced drawします。

WezTermのWebGPU資源再生成候補に対する参考にはなりますが、OpenGLとWebGPUの同期・map/write規則は異なります。まずbind group、texture view、vertex bufferの寿命をRenderStateに持たせ、queue writeまたはring bufferに置換する独自設計とし、GPU capture、frame time、device lost、resize、atlas evictionを検証します。

判定: **B（再利用の原則は有力、backend固有の実装は別設計）**。

## 参照

- [damage API](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/term/mod.rs#L137-L216)
- [ring storage](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/grid/storage.rs#L9-L70)
- [PTY event loop](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty_terminal/src/event_loop.rs#L104-L171)
- [render snapshotとlock解放](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty/src/display/mod.rs#L775-L878)
- [OpenGL buffer/batch再利用](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/alacritty/src/renderer/text/glsl3.rs#L27-L144)
- [長期維持された改善の履歴](https://github.com/alacritty/alacritty/blob/1b2b36a64e88068ad02c95fad00ee2fad31c00bf/CHANGELOG.md#L1409-L1451)
