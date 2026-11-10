# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- git-cliff: end of header -->

## [0.1.0] - 2026-11-10

### Added

- Implement SILV v0.1.0 Kibana-like log viewer (TUI and grep-like modes)
- Guided filter wizard with field autocomplete; fix detail menu Enter

### Fixed

- Report the correct context for the terminal requirement
- Stop leaked mouse sequences and hint the search hotkey
- Swallow ConEmu M-led mouse motion bursts in the key filter
- Auto-disable mouse capture on ConEmu-class consoles that leak mouse bytes
- Request click and wheel mouse reporting only, drop motion tracking
- Auto-disable mouse on JetBrains embedded terminals, dedup filter prompts

### Documentation

- Add usage guide, key map, filter grammar and static-link matrix
- Add changelog via git-cliff

### Testing

- Add end-to-end CLI suite (25 cases) and sample log fixtures

### Miscellaneous Tasks

- Add project scaffolding (gitignore, rustfmt, agents rules, license)
- Add static release build scripts for windows, linux, macos
- Enforce LF line endings
- Add git-cliff configuration


## [Unreleased]

## [0.1.0] - 2026-11-10

### Added

- SILV v0.1.0: Kibana-like log viewer with interactive TUI and grep-like stream modes
- Guided filter wizard with field autocomplete

### Fixed

- Correct context reported for the terminal requirement
- Leaked mouse sequences are filtered out, and the search hotkey is hinted
- ConEmu `M`-led mouse motion bursts are swallowed in the key filter
- Mouse capture is auto-disabled on ConEmu-class consoles that leak mouse bytes
- Mouse reporting is limited to click and wheel events, motion tracking dropped
- Mouse is auto-disabled in JetBrains embedded terminals, filter prompts deduplicated
- Detail menu no longer exits on Enter

### Testing

- End-to-end CLI suite (25 cases) with sample log fixtures

### Documentation

- Usage guide, key map, filter grammar and static-link matrix

### Miscellaneous Tasks

- Project scaffolding (gitignore, rustfmt, agents rules, license)
- Static release build scripts for windows, linux, macos
- Enforce LF line endings
- Changelog and git-cliff configuration

[Unreleased]: https://github.com/chamoretto/silv/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/chamoretto/silv/releases/tag/v0.1.0
