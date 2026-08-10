# Fork development workflow

This fork keeps optimization work and Kitty graphics protocol work as small,
independently buildable commits. Topic commits are promoted to `main` with
`git cherry-pick`; topic branches are never merged into `main`.

## One-time setup

Install and activate [mise](https://mise.jdx.dev/), then run:

```sh
mise trust
mise install
mise exec -- lefthook install
mise run commit-check
```

`mise.toml` pins the stable Rust compiler used for builds, a dated nightly Rust
compiler used only by rustfmt, and Lefthook. `mise run commit-check` checks
formatting and builds the four primary binaries from the upstream Makefile:

- `wezterm`
- `wezterm-gui`
- `wezterm-mux-server`
- `strip-ansi-escapes`

### Windows prerequisite

The vendored OpenSSL build requires Perl. The Perl backend currently registered
by mise does not publish Windows binaries, so install Strawberry Perl and open a
new terminal before running the setup above:

```powershell
winget install --id StrawberryPerl.StrawberryPerl --exact
```

The first build compiles vendored OpenSSL and can take several minutes. Later
commit checks reuse Cargo's incremental build cache.

## Start a focused branch

Begin from an up-to-date `main` and name the branch after one narrow concern:

```sh
git switch main
git pull --ff-only origin main
git switch -c feature/kitty-placeholder-parser
```

Use these branch prefixes:

- `feature/` for Kitty protocol capabilities
- `perf/` for measured optimizations
- `fix/` for correctness fixes
- `infra/` for build and development tooling

Do not combine parser changes, terminal state changes, rendering, and unrelated
cleanup in one commit. A typical Kitty Unicode placeholder series should split
the protocol parser/data model, cell placement, rendering, tests, and docs into
separate commits.

## Create each commit

Stage only one logical change and inspect exactly what will be recorded:

```sh
git add -p
git diff --cached
git commit -m "term: parse kitty unicode placeholders"
```

The pre-commit hook runs `mise run commit-check` for every commit, even for
documentation-only changes. It also rejects direct commits on `main`; a
cherry-pick in progress is the only allowed way to add a commit there.

If a commit changes behavior, include its regression test in the same commit.
Each commit must be understandable, buildable, and safe to cherry-pick alone.

## Rewrite a topic branch

Rebase or amend only on the topic branch. Commits created with `git commit` or
`git commit --amend` pass the pre-commit build automatically. After resolving a
rebase conflict, run the same check once on the resolved snapshot:

```sh
git rebase -i main
mise run commit-check
```

Do not use `cargo build --workspace --all-targets` as the commit gate yet. The
current upstream tree's `wezterm-char-props` benchmark target lacks its
`criterion` and `termwiz` dependencies, so that command fails before fork
changes are applied.

## Promote commits to main

List the verified commits in application order, then cherry-pick only the
commits intended for release:

```sh
git log --reverse --oneline main..HEAD
git switch main
git pull --ff-only origin main
git cherry-pick <oldest-sha> <next-sha> <newest-sha>
mise run commit-check
git push origin main
```

Keep the topic branch until the `main` push succeeds. If a cherry-pick conflicts,
resolve it on `main`, finish the cherry-pick, and rerun `mise run commit-check`
before pushing because the resolved snapshot differs from the verified topic
commit.

Bypassing hooks with `LEFTHOOK=0` is reserved for repairing the hook setup. Any
commit created that way must pass `mise run commit-check` before it is pushed or
cherry-picked.
