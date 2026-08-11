# 14 PTY読取・socketpair・parser pipeline

## 対象と追加調査

mux/src/lib.rs:283-345 のread_from_pane_ptyはpaneごとにblocking read thread、socketpair、parser threadを作り、PTY→socketpair→parserのコピーを行います。pane数に比例してthread、FD、context switchが増えます。

## 壊してはいけない動作

- paneごとのPTY出力順序、backpressure、EOF、parser error、exit behavior。
- slow parser/slow consumer時にメモリが無制限に増えないこと。
- OSごとのblocking readとshutdown、banner、close-on-clean-exit。

## 追加テスト

- 複数paneの同時burst、slow consumer、巨大burst、EOF、非UTF-8、parser error、pane終了。
- 各paneの入力順序と出力順序をsequence numberで比較。
- thread数、FD数、queue bytes、read-to-render latency、RSSを計測。

## 実装方針

いきなり共有event loopへ統合せず、まずsocketpair bytesとparser latencyをmetrics化します。次にbounded channel/worker poolを試し、pane単位の順序テストを通します。リスクは非常に高です。
