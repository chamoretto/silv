# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- git-cliff: end of header -->

## [Unreleased]

## [0.1.1] - 2026-11-10

### Fixed

- *(ci)* Hand the generated notes file to the release, not the path string

### Documentation

- Deduplicate the 0.1.0 changelog section
- Move commit and semver specifications into docs/
- Add interactive TUI screenshot and the maintainer release routine

### Miscellaneous Tasks

- Add image binary rules and screenshot asset guide

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
- Rewrite changelog in keep a changelog notation
- Add AI disclosure, badges and contributing guide
- Keep published docs free of internal path references

### Testing

- Add end-to-end CLI suite (25 cases) and sample log fixtures

### Miscellaneous Tasks

- Add project scaffolding (gitignore, rustfmt, agents rules, license)
- Add static release build scripts for windows, linux, macos
- Enforce LF line endings
- Add git-cliff configuration
- Track Cargo.lock and allow .github in version control
- Add CI, release workflow and repository templates
- Map git-cliff groups to keep a changelog notation
- Add repository metadata for github publishing
- Track Cargo.lock for reproducible release builds


[Unreleased]: https://github.com/chamoretto/silv/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/chamoretto/silv/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/chamoretto/silv/releases/tag/v0.1.0

