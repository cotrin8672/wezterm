# 01 WebGPU資源の毎フレーム再生成

## 対象と追加調査

wezterm-gui/src/termwindow/render/draw.rs:26-143 は、フレームごとにsurface texture view、glyph atlas view、linear/nearest bind groupを作ります。layer×3の非空vertex bufferごとにrender passとuniform bind groupを生成し、vertices.webgpu_mut().recreate()を呼びます。実体は wezterm-gui/src/renderstate.rs:238-247 のmapped-at-creation buffer作成です。

負荷は文字数、layer数、cursor blink/animationで増えます。bind group cacheだけでもGPU allocatorとCPU descriptor生成を削減できる可能性があります。

## 壊してはいけない動作

- atlas再作成後に古いtexture view/bind groupを再利用しない。
- milliseconds、projection、foreground HSB、sampler filterをフレームごとに更新する。
- GPU submit完了前のbufferを上書きしない。
- render passのclear/load順、z-index、blend、空layerを維持する。

## 追加テスト

- WebGPU/GLで通常文字、CJK/emoji、cursor、blink、画像、透過背景を同じ入力で描画する。
- resize、DPI変更、font reload、atlas拡張後に新glyphが表示されること。
- wgpu traceでbind group数、buffer作成数、CPU/GPU frame timeを比較する。

## 安全な実装順

1. atlas generationとbind group生成回数をmetrics化。
2. atlas generationをキーにbind group cacheだけを導入。
3. 動的uniformをwrite bufferへ移行。
4. 最後にvertex bufferをリング化する。

破壊リスクは高です。GL pathとGPU lifetimeの変更を同じPRに入れません。
