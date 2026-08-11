# 06 CSI bulk editと左右マージンscroll

## 対象と追加調査

term/src/terminalstate/mod.rs:2206-2220,2273-2289 のDeleteCharacter/InsertCharacterは1セルずつ処理します。wezterm-surface/src/line/line.rs:1018-1095 の各処理はVec remove/insert、hyperlink/zone invalidation、seqno更新を伴います。term/src/screen.rs:566-650 の左右マージンscrollも行ごとにセルをcloneします。

## 壊してはいけない動作

- ICH/DCHの右端で捨てるセル、wide cell補助セル、wrap、cursor。
- DECLRMMの左右外側を変更しないこと、Bidi、blank attrs、dirty seqno。
- hyperlink/semantic zone/Kitty placeholderのinvalidation範囲。

## 追加テスト

- CSI n@/nP/nX/nDをn=0,1,cols-1,cols,cols+1で比較。
- double-width、explicit/implicit hyperlink、semantic zone、Kitty placeholderを含む行。
- term/src/test/mod.rs:568-650 の左右マージンscrollに外側列不変assert。
- CSI 1000@/CSI 1000Pのthroughput、alloc、p99を正しさテストと分離して測る。

## 実装方針

一括range shiftを追加しても、小さいn/特殊セルは既存経路へfallbackします。移行中は旧実装と新実装のLine結果を差分比較します。破壊リスクは最高クラスです。
