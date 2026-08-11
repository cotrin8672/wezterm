# 17 SSHのpoll待ち

## 対象と追加調査

mux/src/ssh.rs:522-555 のpoll_inputはdeadlineを計算しますが、poll自体には常にDuration::from_millis(200)を渡します。waitが1〜10msでも、入力がない場合に最大200ms単位で戻りが遅れる可能性があります。

## 壊してはいけない動作

- wait=Noneの無期限待ち、wait=0の即時戻り、resize通知、EOF。
- 入力queueの順序とparserへのbyte chunk境界。
- socket descriptorのnon-blocking設定とOS差。

## 追加テスト

- wait=0/1ms/10ms/200ms/None、入力到着直後、resize、EOF。
- poll timeoutをmin(残り時間, 200ms)にした場合のp50/p99を比較。
- slow networkや短い入力chunkで入力を失わないこと。

## 実装方針

まずdeadline残り時間を使う局所変更にし、eventfd/waker導入は別PRにします。優先度は中、リスクは中です。
