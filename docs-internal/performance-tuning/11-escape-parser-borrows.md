# 11 escape-parserのbyte loop borrow

## 対象と追加調査

wezterm-escape-parser/src/parser/mod.rs:130-190 のparse_firstは各byteでRefCell borrowとclosureを行い、parse_first_as_vecもstate.borrow_mutをbyteごとに作ります。これはparserの全入力経路ではなく、最初のactionを取り出す特殊経路です。

## 壊してはいけない動作

- chunk境界を跨ぐstate machine、ground復帰、最初のactionのindex。
- callback中の再入を禁止/許可する現在のborrow規則。
- parse_firstとparse_first_as_vecのaction順序と消費byte数。

## 追加テスト

- 同じ入力を1byteずつ、ランダムchunk、全byteで与えaction列と消費長を比較。
- ESC、CSI、OSC、DCS、APC、UTF-8途中で停止するケース。
- callbackが複数actionを発生させるsequenceのfirst/vec一致。

## 実装方針

最初にstateを1回だけ借用する案を検討し、再入が必要な公開APIなら適用しません。criterionで短い入力と長い入力を分けて測ります。リスクは中です。
