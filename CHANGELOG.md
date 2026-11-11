# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- git-cliff: end of header -->

## [0.1.4] - 2026-11-11

### Added

- *(tui)* Map keyboard layouts to QWERTY bindings on every platform ([450291e](https://github.com/chamoretto/silv/commit/450291eec1aa68ca62ab7034a78182e26ed99405))
- *(tui)* Filter undo/redo keys and chip rows that wrap ([707c40b](https://github.com/chamoretto/silv/commit/707c40bce66ce92d90ba4836f96603ac50ce0008))
- *(tui)* Redo with Ctrl+Y and history keys in the prompt line ([480f225](https://github.com/chamoretto/silv/commit/480f225e374640e7c4f57e75bc5f1ac37ab82779))
- *(tui)* S focuses the field header for keyboard sorting ([69f72a7](https://github.com/chamoretto/silv/commit/69f72a70c3119c130d785afbc59261fe466ac141))

### Fixed

- *(tui)* Match key bindings on the physical QWERTY key for non-Latin layouts ([57906de](https://github.com/chamoretto/silv/commit/57906de4654cc2db988cc7ea09ce02d953321f38))
- *(tui)* Wrap prompt lines and separate them from the rows ([e0a9e0e](https://github.com/chamoretto/silv/commit/e0a9e0edaa4c48f41e8e9502678089c643ae6355))
- *(changelog)* One Unreleased heading and a valid Unreleased link ([5ec772a](https://github.com/chamoretto/silv/commit/5ec772a807231c0aa30550a113d200452bb0747f))
- *(keymap)* Keep macOS layout sources alive while translating ([55b06f0](https://github.com/chamoretto/silv/commit/55b06f0afafd2dce83a0015d4e336a13c4dff49f))
- *(keymap)* Probe the macOS layout on the main thread ([#5](https://github.com/chamoretto/silv/issues/5)) ([64e0657](https://github.com/chamoretto/silv/commit/64e065715dd9ed16091478eac9f4b73cff9e55ca))
- *(cliff)* Keep squashed PR commits in the changelog ([2d14cbf](https://github.com/chamoretto/silv/commit/2d14cbf06b290b3133d2a215ba12c877c8b8ac63))
- *(changelog)* Regenerated CHANGELOG.md ([#6](https://github.com/chamoretto/silv/issues/6)) ([840d34c](https://github.com/chamoretto/silv/commit/840d34c4446ff5fe1bf137833ace4b245fec02cf))

### Documentation

- Update the interactive TUI screenshot ([ec1a42f](https://github.com/chamoretto/silv/commit/ec1a42fdac85620ed5c7a543bba25a41e8b24a99))

### Testing

- *(tui)* Match the prompt line test to the reworded prompts ([e782bd3](https://github.com/chamoretto/silv/commit/e782bd3b07d72f30513ec958e7357c0caa1fb8c2))

### Miscellaneous Tasks

- Add the local three-platform gate script ([11dd4f6](https://github.com/chamoretto/silv/commit/11dd4f60b4db9974385ffd204704cefe5b6e4f96))

## [0.1.3] - 2026-11-10

### Fixed

- *(tui)* Key tokens inherit the default foreground on their background ([53265ab](https://github.com/chamoretto/silv/commit/53265abae229ce13f6bc17317e8037267e31738e))

## [0.1.2] - 2026-11-10

### Added

- *(tui)* Color operators, split chip lines, highlight matches, reorder columns ([41e7733](https://github.com/chamoretto/silv/commit/41e77338a297f61a2a2d9a0b5e96cbfc508c6efa))
- *(tui)* Prompt line, chip and detail prompts, row value picker ([2113963](https://github.com/chamoretto/silv/commit/21139637c86e2886fe1785d367e75d69b07af39b))
- *(tui)* Visible pick block and key-token prompt highlighting ([e98c0bb](https://github.com/chamoretto/silv/commit/e98c0bb693d69165944b991a5338d59d6cb315dd))

### Fixed

- *(tui)* Space toggles the focused field in the picker ([3a2e7d8](https://github.com/chamoretto/silv/commit/3a2e7d8dc17dad3afbe1eada4b94c9f9774dd2e8))

### Documentation

- Note the PowerShell form of the lock refresh command ([63f8798](https://github.com/chamoretto/silv/commit/63f87982c6370f2d023ab15d18e8352a03720811))
- Order the release routine so the cargo gate refreshes the lock ([1823dbd](https://github.com/chamoretto/silv/commit/1823dbdccd1df5f4562fb5d4e5a51ff0d61c0c66))

### Miscellaneous Tasks

- *(changelog)* Commit-sha entries and post-commit auto-update hook ([f04bf03](https://github.com/chamoretto/silv/commit/f04bf03cac2ac505c2614702db564a2cab32e578))

## [0.1.1] - 2026-11-10

### Fixed

- *(ci)* Hand the generated notes file to the release, not the path string ([61b973e](https://github.com/chamoretto/silv/commit/61b973ea2c7310dd51c9a4d0900aa9060e811312))

### Documentation

- Deduplicate the 0.1.0 changelog section ([1c26697](https://github.com/chamoretto/silv/commit/1c266973c856e47d1be99776bbbeb22a31f41866))
- Move commit and semver specifications into docs/ ([6ebeebd](https://github.com/chamoretto/silv/commit/6ebeebd68fb43af3e1a0550da31c48c914826dbc))
- Add interactive TUI screenshot and the maintainer release routine ([2cb0aab](https://github.com/chamoretto/silv/commit/2cb0aabfb923aea1d7eede88e1abed3bec1c6d57))

### Miscellaneous Tasks

- Add image binary rules and screenshot asset guide ([eaadec1](https://github.com/chamoretto/silv/commit/eaadec1e5d62646de22b86e4adc604354cf139b6))
- Emit the unreleased section and version link refs from cliff config ([002d454](https://github.com/chamoretto/silv/commit/002d4542dbccec275a9bdf44e945a4eb04015c19))

## [0.1.0] - 2026-11-10

### Added

- Implement SILV v0.1.0 Kibana-like log viewer (TUI and grep-like modes) ([b341c6e](https://github.com/chamoretto/silv/commit/b341c6e6a675190d38b9e0ffa229730898795daa))
- Guided filter wizard with field autocomplete; fix detail menu Enter ([c6d5821](https://github.com/chamoretto/silv/commit/c6d5821e2060b6bd547fc7a072d61632aa89539c))

### Fixed

- Report the correct context for the terminal requirement ([1542ac4](https://github.com/chamoretto/silv/commit/1542ac4506b8f8e03ac218c2e52ab45c2c5be0d5))
- Stop leaked mouse sequences and hint the search hotkey ([a400ca9](https://github.com/chamoretto/silv/commit/a400ca95339e6021b765b6e1e71bc892c6297c20))
- Swallow ConEmu M-led mouse motion bursts in the key filter ([03d83c8](https://github.com/chamoretto/silv/commit/03d83c8e849238f0561f8470e172de770eeee0f9))
- Auto-disable mouse capture on ConEmu-class consoles that leak mouse bytes ([d15355d](https://github.com/chamoretto/silv/commit/d15355dc480718564db4481fe8f73cfc0852a72f))
- Request click and wheel mouse reporting only, drop motion tracking ([9e01d2c](https://github.com/chamoretto/silv/commit/9e01d2c3378e840476ef785efbdff31d4f297712))
- Auto-disable mouse on JetBrains embedded terminals, dedup filter prompts ([c107556](https://github.com/chamoretto/silv/commit/c107556685195deb61fc1462c44f717488bbd66f))

### Documentation

- Add usage guide, key map, filter grammar and static-link matrix ([8bf0dd9](https://github.com/chamoretto/silv/commit/8bf0dd9fde5c3d9ee53b8dae7f7de336143210d8))
- Add changelog via git-cliff ([7965808](https://github.com/chamoretto/silv/commit/79658087cf9cdf2c4602cce37bd610e0d2578bbb))
- Rewrite changelog in keep a changelog notation ([a386052](https://github.com/chamoretto/silv/commit/a386052644f683602f119e1e19dd428618559dc8))
- Add AI disclosure, badges and contributing guide ([b8f91a7](https://github.com/chamoretto/silv/commit/b8f91a719bd94cb5b53650f271b8759344d5d3a8))
- Keep published docs free of internal path references ([05e0b08](https://github.com/chamoretto/silv/commit/05e0b08d73d7063ff58fe0452ee62efb4eea8581))

### Testing

- Add end-to-end CLI suite (25 cases) and sample log fixtures ([5051fc9](https://github.com/chamoretto/silv/commit/5051fc991b7993e4165ebb19316c8e8fe7db5427))

### Miscellaneous Tasks

- Add project scaffolding (gitignore, rustfmt, agents rules, license) ([41ea4d7](https://github.com/chamoretto/silv/commit/41ea4d7f1e57e840a965e8435630b6b0e944983d))
- Add static release build scripts for windows, linux, macos ([7ab68dc](https://github.com/chamoretto/silv/commit/7ab68dcb9dddf3469a36c33d39fb8c9a53020635))
- Enforce LF line endings ([674ccf4](https://github.com/chamoretto/silv/commit/674ccf41fe3df87b09d38200b148c44fb623d6f9))
- Add git-cliff configuration ([106e8e0](https://github.com/chamoretto/silv/commit/106e8e016fd7a0bc177057010b177a7d80411010))
- Track Cargo.lock and allow .github in version control ([a4d9977](https://github.com/chamoretto/silv/commit/a4d9977916fe12785bee5c3262cb4a80c42a726e))
- Add CI, release workflow and repository templates ([a92c772](https://github.com/chamoretto/silv/commit/a92c772fcc2c8cb756a375487ad328f8973efb39))
- Map git-cliff groups to keep a changelog notation ([dfd6157](https://github.com/chamoretto/silv/commit/dfd61576349372b2472a251538f004cd5d2ab404))
- Add repository metadata for github publishing ([0ca57f4](https://github.com/chamoretto/silv/commit/0ca57f4f706227d150cf23b56cabe05bcc66da33))
- Track Cargo.lock for reproducible release builds ([24aa87e](https://github.com/chamoretto/silv/commit/24aa87eb3a9574be2f754ea364395ae344bc737a))


[Unreleased]: https://github.com/chamoretto/silv/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/chamoretto/silv/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/chamoretto/silv/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/chamoretto/silv/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/chamoretto/silv/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/chamoretto/silv/releases/tag/v0.1.0

