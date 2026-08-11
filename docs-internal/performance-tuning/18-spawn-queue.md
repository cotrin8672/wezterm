# 18 spawn queueの公平性とwake回数

## 対象と追加調査

window/src/spawn.rs:68-99,162-190 は高優先度queueを空にするまで低優先度queueへ進まず、main loop復帰ごとの処理件数も限定されています。高優先度taskが連続すると低優先度taskが飢餓になり、1件ずつのwake/lock往復も増えます。

## 壊してはいけない動作

- input、paint、shutdownの順序と高優先度の即時性。
- promise完了、例外伝播、キャンセル、main thread affinity。
- low priority taskが最終的に必ず実行されること。

## 追加テスト

- high queue連続投入中にlow queueが一定時間内に実行されること。
- taskがself-spawn、panic、cancelするケース。
- queue長、pop待ち時間、wake回数、main loop占有時間をhistogram化。

## 実装方針

件数または時間budgetとwake coalescingを導入します。公平性を変える場合は決定的なexecutor testを先に追加します。優先度は中です。
