# Screenshots

Screenshot assets for the README, the wiki and GitHub discussions live here.

## Layout and naming

```text
docs/images/
  ui-interactive-sample-jsonl.png   interactive table view, filter chip, sort header
  ui-detail.png                     record detail overlay
  ui-stream.png                     stream mode with highlighted hits
  terminal-<name>.png               environment specific captures
```

Lowercase kebab-case names, no spaces, no dates in file names (git history carries dates).

## Capture recipe (TUI)

- Use a terminal with full mouse and 24-bit color support: Windows Terminal, conhost, GNOME
  Terminal, kitty, iTerm2. ConEmu/MSYS2/JetBrains-embedded consoles disable mouse capture
  (see the README), so wheel/click interactions cannot be shown there.
- Set an 80x24 or 100x30 window, a dark theme, and a font with box-drawing/emoji coverage
  (Cascadia Code, JetBrains Mono).
- Feed sample data: `silv samples/sample.jsonl`, then capture with `Win+Shift+S` (snipping
  tool), `PrtSc`, `macOS: Cmd-Shift-4`, or `PrintScreen` on X11/Wayland.
- For interactions, record a short video (`Win+G` Xbox Game Bar on Windows, `Cmd-Shift-5` on
  macOS). GitHub renders `.mp4` attachments in issues, PRs and release bodies.
- Crop to the terminal client area; keep PNG, aim below 300 KB per image (run `pngquant` if
  larger). No git-lfs needed at this size.

## Referencing from markdown

```markdown
![interactive table view](docs/images/ui-interactive-sample-jsonl.png)

<table>
  <tr>
    <td><img src="docs/images/ui-interactive-sample-jsonl.png" width="360"></td>
    <td><img src="docs/images/ui-detail.png" width="360"></td>
  </tr>
</table>
```

Use relative paths for committed files (GitHub rewrites them to raw URLs), and keep the same
paths working for a `crates.io`/docs.rs rendering by avoiding `./` prefixes.

## Not rendered by GitHub

- Locally committed `.svg` via `<img>`: GitHub blocks it. Use raster, or a shields.io badge URL.
- Drag-and-drop uploads made in issues/PRs/releases are stored on GitHub's asset hosts, not in
  the repository, so they break in README copies and mirrors.
