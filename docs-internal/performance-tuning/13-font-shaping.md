# 13 font shapingとHarfBuzz allocation

## 対象と追加調査

wezterm-font/src/shaper/harfbuzz.rs:346-347 はUTF-8 byte長をglyph clusterのcapacityに使います。wezterm-font/src/lib.rs:181-225 はshapeごとにfallback Mutex、Vec、HashSetを操作します。shape cache miss時のみ頻度が高くなります。

## 壊してはいけない動作

- HarfBuzz clusterのbyte offsetとglyphのcell幅の対応。
- font fallbackの順序、pending fallbackの非同期通知、ClearShapeCache。
- RTL、結合文字、emoji ZWJ、presentation width、synthetic glyphの扱い。

## 追加テスト

- Latin、CJK、結合文字、emoji ZWJ、RTL、font fallback、color emoji、DPI変更。
- cache miss/hitでshape結果とglyph indexが一致すること。
- fallback font追加後に次frameで正しいglyphへ切り替わること。
- 多言語入力のallocation数、peak RSS、shape latencyを比較。

## 実装方針

まずglyph数に基づくreserve、thread-local scratch Vec、fallbackがある時だけlockする変更から始めます。HarfBuzz cluster計算そのものは変更せず、画像比較/golden結果で確認します。リスクは中〜高です。
