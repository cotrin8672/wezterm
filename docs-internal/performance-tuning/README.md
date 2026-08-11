# パフォーマンス調査（項目別）

このディレクトリは、性能改善候補を1項目ずつ調査した記録です。各ページは「観測したコード」「壊してはいけない動作」「追加テスト」「計測と実装順」を分離して記載しています。ここではまだコード変更を行っていません。

## 描画

- [01 WebGPU資源](01-webgpu-resources.md)
- [02 paintとdirty region](02-paint-dirty-region.md)
- [03 GL draw call](03-gl-draw-batching.md)
- [04 mousemoveとフレーム制御](04-mouse-frame-pacing.md)

## terminal/surface

- [05 printとscrollback圧縮](05-terminal-print-scrollback.md)
- [06 CSI bulk edit](06-csi-bulk-edit.md)
- [07 Kitty placeholder](07-kitty-placeholder.md)
- [08 hyperlink scan](08-hyperlink-scan.md)
- [09 Terminal Mutex](09-terminal-lock-snapshot.md)

## parser/font/cache

- [10 vtparse buffer](10-vtparse-buffers.md)
- [11 escape-parser borrow](11-escape-parser-borrows.md)
- [12 shape cache](12-shape-cache.md)
- [13 font shaping](13-font-shaping.md)

## I/O・実行基盤

- [14 PTY pipeline](14-pty-pipeline.md)
- [15 設定reload](15-config-reload.md)
- [16 Linuxプロセス情報](16-proc-info.md)
- [17 SSH poll](17-ssh-poll.md)
- [18 spawn queue](18-spawn-queue.md)
- [19 ベンチマーク計画](19-benchmark-plan.md)

## Kitty / Alacritty / Ghostty 比較

- [20 競合実装との比較マトリクス](20-competitor-comparison.md)
- [21 Kitty の実装確認](21-kitty-implementation.md)
- [22 Alacritty の実装確認](22-alacritty-implementation.md)
- [23 取り込み候補の分類](23-adoption-classification.md)
- [24 Ghostty の実装確認](24-ghostty-implementation.md)

優先度は、まず意味論を変えない割り当て削減と計測追加、次にterminalのbulk edit/hyperlink/Kitty、最後にGPU資源共有・dirty region・PTY構成変更の順です。
