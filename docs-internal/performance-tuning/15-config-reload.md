# 15 設定reload・configuration Mutex・color scheme

## 対象と追加調査

PTY/parser経路ではreadごとにconfiguration取得が行われ、config側のgetはMutex lockとArc cloneを伴います。config/src/config.rs:1328-1395,1450-1478 はreload時にcolor scheme directoryを走査し、各TOMLを同期read/parseします。

## 壊してはいけない動作

- reload後の設定反映タイミングとreaderが見るsnapshot。
- 外部schemeの追加/削除/変更、同名schemeの優先順位、構文エラー。
- exit behaviorやPTY設定など、設定Mutex経由で更新される値。

## 追加テスト

- reload中にPTY出力を流し、chunkごとの設定snapshotが不自然に混ざらないこと。
- 外部scheme追加/削除/変更、壊れたTOML、同名scheme。
- startup/reload時間、filesystem syscall数、Mutex waitを記録。

## 実装方針

設定のgenerationとimmutable Arc snapshotをparser threadへ渡し、generation変更時だけ更新する案を検討します。color schemeはmetadata cacheまたは選択schemeのlazy parseから始めます。リスクは中〜高です。
