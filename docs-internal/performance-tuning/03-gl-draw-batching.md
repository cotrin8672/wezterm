# 03 GL draw callとUniformBuilder

## 対象と追加調査

wezterm-gui/src/termwindow/render/draw.rs:237-272 はlayer×3の非空bufferごとにframe.drawし、UniformBuilderとsamplerを構築します。draw call数はlayer、背景画像、glyph atlas filter、z-indexの組み合わせで増えます。

## 壊してはいけない動作

- 背景→glyph→overlay→cursorの描画順とz-indexを変えない。
- linear/nearest sampler、subpixel AA、foreground HSB、blink時刻を維持する。
- alpha blendとtransparent windowの色合いを変えない。

## 追加テスト

- 通常文字、色付きemoji、画像、selection、cursor、半透明背景、下線をGLで目視比較。
- layer数0/1/3、空buffer、atlas再作成のケース。
- frameごとのdraw call数、CPU時間、GPU frame timeを記録。

## 実装方針

Uniformのフレーム不変部分をcacheし、時刻やprojectionだけ更新するのが安全です。quadのbatch統合は、同じtexture/filter/blend/z-indexの連続範囲だけに限定し、golden screenshotを通過させます。優先度は中です。
