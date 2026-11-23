# Topiary Markdown

A standalone formatter for CommonMark and GitHub Flavored Markdown. It tidies
headings, lists, thematic breaks, block spacing, emphasis, and GFM tables while
keeping your existing prose line breaks. Code blocks, raw HTML, front matter,
and recognized LaTeX stay untouched.

You don't need Node or Prettier to run it, and it doesn't try to match Prettier
byte-for-byte. While the project is still pre-1.0, formatting style and CLI
behavior can change.

## What it formats

- **Headings and block spacing:** cleans up ATX heading spacing and closing `#`
  runs, normalizes blank lines between blocks, adds a trailing newline when
  missing, and uses `***` for thematic breaks that need normalization.
- **Lists:** normalizes list markers (defaulting unordered bullets to `-` and
  numbering ordered items sequentially) and aligns list continuation
  indentation.
- **GFM tables:** aligns and pads table columns while leaving cell contents
  alone.
- **Inline style and whitespace:** normalizes safe emphasis to `_…_` and strong
  emphasis to `**…**`, trims stray trailing spaces in prose, and preserves
  existing prose wrapping and two-space hard breaks.
- **Protected regions:** leaves code blocks, raw HTML, front matter, reference
  definitions, and recognized inline or display math untouched, including
  dollar-delimited math and unambiguous `\(…\)` and `\[…\]` blocks.

The formatter can skip a local edit if it would change the parsed structure or
protected bytes. If the final output fails that check, it returns an error.

## Install

### Download a binary

Look for a `.tar.gz` under [Releases](https://github.com/InnocentZero/topiary-markdown-fmt/releases)
or under **Artifacts** on a successful [CI run](https://github.com/InnocentZero/topiary-markdown-fmt/actions).
Both have Linux (glibc, built on Ubuntu 24.04) and macOS builds for x86-64 and
ARM64. CI artifacts come in a ZIP containing the `.tar.gz`. Unpack the archive
and put `topiary-markdown` on your `PATH`, for example in `~/.local/bin`.

### Nix

Nix downloads the binary from a GitHub Release, **not** from a CI run's
Artifacts section. A green CI run is enough for a manual download, but the Nix
commands below need both a published Release and the `nix-release` branch. If
either is missing, use a CI artifact or build from source for now.

```sh
nix profile add github:InnocentZero/topiary-markdown-fmt
nix run github:InnocentZero/topiary-markdown-fmt -- --help
```

The flake uses the `nix-release` branch to find the archive and its checksum.
It verifies the download and patches the Linux binary to run on NixOS. It
doesn't compile Rust.

If you manage your setup with a NixOS or Home Manager flake, add an input named
`topiary-markdown` and pull in
`inputs.topiary-markdown.packages.${system}.default`. To pick up a newer release
later, run `nix flake update topiary-markdown` in your flake.

### Build from source

You can build straight from this repository if you have stable Rust and a C
compiler installed:

```sh
cargo build --locked --release
cargo test --locked
```

That puts the binary at `target/release/topiary-markdown`.

## Usage

```sh
topiary-markdown < input.md > formatted.md
topiary-markdown --check input.md
topiary-markdown --write input.md
```

With no arguments, it reads from stdin and writes to stdout. `--check` exits
with `1` when formatting would change the file; it only checks whether the
formatter would make edits and doesn't act as a style linter. Parsing or
verification errors exit with `2`, and `--write` leaves the file untouched when
that happens.

### Neovim

If you use [Conform.nvim](https://github.com/stevearc/conform.nvim) and have
`topiary-markdown` on your `PATH`, add this to your Conform config:

```lua
require("conform").setup({
  formatters_by_ft = { markdown = { "topiary_markdown" } },
  formatters = {
    topiary_markdown = { command = "topiary-markdown", stdin = true },
  },
})
```

Run `:lua require("conform").format()` to format the current buffer. Everything
goes over stdin and stdout, so Neovim stays in charge of buffer state and
saving. Don't map MDX or other Markdown dialects with custom syntax to this
formatter.

## How it works

Under the hood, Topiary and Tree-sitter queries propose block separators, and a
Rust adapter applies them at confirmed block boundaries alongside local
Markdown edits for tables, lists, and emphasis. Before returning output, an
independent CommonMark/GFM parser checks that the AST, reference definitions,
and protected regions still match the input.

## Releases

GitHub Actions builds and tests Linux and macOS binaries on both architectures.
Pushing a `vX.Y.Z` tag publishes those archives and their checksums to GitHub
Releases and updates the `nix-release` metadata branch. A consumer's Nix lock
file pins a commit on that metadata branch, so it stays on that tagged release
as long as the release assets remain available.

## Corpus runs

The development run passed all checks on 59,156 of 60,000 files. The corpus is
mostly documentation, with many files from large Microsoft docs repositories.
This is a development benchmark, not a result on unseen files. The manifest is
at `corpus/manifest.jsonl`; input files and results live outside version control.

To launch the local dashboard:

```sh
npm ci --ignore-scripts
python3 scripts/dashboard.py          # http://127.0.0.1:56809/
```

For a scored run, point the benchmark script at immutable formatter,
style-checker, and verifier binaries, pin the style profile, and pass the
formatter revision:

```sh
python3 scripts/bench.py my-run \
  --revision "$REV" --binary "$FORMATTER" \
  --style-profile prettier-like-v6 --style-binary "$STYLE" \
  --verifier-binary "$VERIFIER"
```

A file passes only if it satisfies the concrete style profile, preserves its
AST and protected bytes, produces identical bytes in a separate formatter
process, and stays byte-identical on a second pass. Unsupported inputs,
timeouts, and preservation failures all fail. Prettier 3.10.1 (`proseWrap:
preserve`, embedded formatting disabled) runs separately for exact-match and
line-distance tracking without affecting pass/fail verdicts.

Between 256-fix checkpoints, build a targeted set with
`scripts/select_affected.py` and run with `--before`, `--affected`, `--sha`, and
`--sample 100 --seed …`. That compares both binaries under the same immutable
oracles and flags any hard check that regresses. At every 256th actual formatter
fix, run `scripts/checkpoint.py` against the full manifest to localize
regressions along linear Jujutsu history. The dashboard shows one dot per pinned
file (unchecked files stay gray during targeted runs), and detailed outputs live
under `measurements/<run>/`.
