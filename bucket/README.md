# WezTerm Kitty Scoop bucket

This repository also contains a small Scoop bucket for the Windows build with
Kitty Unicode placeholder support.

```powershell
scoop bucket add wezterm-kitty https://github.com/cotrin8672/wezterm
scoop install wezterm-kitty
```

The `wezterm-kitty.json` manifest is generated and committed by the Windows
release workflow after each `20*` tag. The manifest points at the matching
SHA256-verified ZIP in the GitHub Release.
