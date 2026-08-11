# 10 vtparse OSC/APC buffer

## 対象と追加調査

vtparse/src/lib.rs:527-645 はOSC/APC開始・clearのたびにbuffer.clear後のshrink_to_fitを呼びます。OSC/APCを高頻度に送るとmalloc/freeと再確保が繰り返されます。

## 壊してはいけない動作

- OSCの最大parameter数、C1/ST/BEL/CAN/SUB終端を維持する。
- APC payloadの所有権とApcEnd後の再利用を変えない。
- shrink削除で無制限に容量を保持しない。

## 追加テスト

- 連続OSC、空payload、巨大payload後の小payload、BEL/ST終端。
- APCを複数chunkに分割し、dispatch payloadが完全一致すること。
- 既存vtparse/src/lib.rsのOSC/APCテストにcapacity上限のassertを追加。
- 反復時のallocation回数、capacity、RSSを計測。

## 実装方針

clear時のshrinkを無条件に削除するのではなく、容量が閾値を超えた場合だけ縮小する案を比較します。意味論を変えないため、優先度は高、リスクは低〜中です。
