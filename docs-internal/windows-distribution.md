# Windows release and Scoop installation

The fork's Windows release workflow is
`.github/workflows/release-windows-fork.yml`. It runs for tags beginning with
`20`, builds the GUI, CLI, mux server, and `strip-ansi-escapes`, packages the
portable ZIP and Inno Setup installer, computes SHA256 files, and creates a
GitHub Release.

Create a release from a clean commit with:

```bash
git tag 20260811-123456-<short-sha>
git push origin 20260811-123456-<short-sha>
```

After the workflow succeeds, it commits `bucket/wezterm-kitty.json` to
`main`. Users can then install the portable build with:

```powershell
scoop bucket add wezterm-kitty https://github.com/cotrin8672/wezterm
scoop install wezterm-kitty
```

The manifest points at the matching release ZIP and verifies its SHA256 hash.
