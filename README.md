# Topiary Markdown

A standalone formatter for CommonMark and GitHub Flavored Markdown. No Node or
Prettier needed; pre-1.0, so formatting style and CLI behavior may still change.

## What it formats

- **Headings and blocks:** cleans up ATX headings, blank lines between blocks,
  trailing newlines, and thematic breaks (`***`).
- **Lists and tables:** normalizes unordered bullets to `-`, numbers ordered
  lists sequentially, aligns continuation indentation, and pads GFM table
  columns without editing cell contents.
- **Inline and prose:** normalizes safe emphasis to `_…_` and `**…**`, trims
  stray trailing spaces, and keeps your existing prose line breaks and two-space
  hard breaks.
- **Left untouched:** code blocks, raw HTML, front matter, reference
  definitions, and recognized inline or display math (dollar-delimited math and
  unambiguous `\(…\)` / `\[…\]`).

If an edit would alter the parsed document or protected regions, it is skipped;
if the output still fails verification, the formatter exits with an error.

## Install

### Prebuilt binary

Download a `.tar.gz` for Linux (glibc) or macOS (x86-64, ARM64) from
[Releases](https://github.com/InnocentZero/topiary-markdown-fmt/releases),
unpack it, and place `topiary-markdown` on your `PATH`.

### Nix

Installs the prebuilt release binary without compiling Rust:

```sh
nix profile add --no-write-lock-file github:InnocentZero/topiary-markdown-fmt
nix run --no-write-lock-file github:InnocentZero/topiary-markdown-fmt -- --help
```

In a NixOS or Home Manager flake, add this repository as input
`topiary-markdown` and set `inputs.topiary-markdown.inputs.nixpkgs.follows = "nixpkgs"`.
Add `(inputs.topiary-markdown.lib.mkPackage pkgs)` to your package list. Update the
formatter with `nix flake update topiary-markdown`.

### From source

Requires stable Rust and a C compiler:

```sh
cargo build --locked --release   # target/release/topiary-markdown
```

## Usage

```sh
topiary-markdown < input.md > formatted.md
topiary-markdown --check input.md
topiary-markdown --write input.md
```

Reads from stdin and writes to stdout by default. `--check` exits `1` if
formatting would change the file. Parse or verification errors exit `2` and
leave `--write` files untouched.

### Neovim

With [Conform.nvim](https://github.com/stevearc/conform.nvim) and
`topiary-markdown` on your `PATH`:

```lua
require("conform").setup({
  formatters_by_ft = { markdown = { "topiary_markdown" } },
  formatters = { topiary_markdown = { command = "topiary-markdown", stdin = true } },
})
```

Run `:lua require("conform").format()` to format the current buffer. Don't map
MDX or other custom-syntax Markdown dialects to this formatter.
