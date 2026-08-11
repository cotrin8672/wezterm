# 16 Linuxプロセス情報の全走査

## 対象と追加調査

procinfo/src/linux.rs:31-103 のwith_root_pidは/procを全PID列挙し、各PIDのstat、exe、cwd、cmdlineを読む場合があります。mux/src/localpane.rsのforeground process取得から呼ばれると、pane操作や表示更新のたびにO(全PID) I/Oになり得ます。

## 壊してはいけない動作

- 子孫process tree、foreground pid、cwd、argvの意味。
- PID再利用をstarttimeで区別すること。
- 権限エラー・終了競合を「processなし」と誤認しないこと。
- Cached/Immediate fetchの既存TTLとclose判定。

## 追加テスト

- 子process tree、PID再利用、権限拒否、process終了競合をfixture/mockでテスト。
- 10、1000、10000 process相当のfixtureでlatencyとsyscall数を比較。
- Cached/ImmediateのTTL、foreground process変更検出、cwd変更を確認。

## 実装方針

対象PIDと親子関係だけを読む、または非同期cacheを更新する案を比較します。cache導入時はPID+starttimeをキーにし、close判定はImmediateを維持します。リスクは中〜高です。
