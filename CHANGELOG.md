# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- git-cliff: end of header -->

## [Unreleased]

### Fixed

- *(cliff)* Keep squashed PR commits in the changelog
- *(changelog)* Regenerated CHANGELOG.md

## [0.1.4] - 2026-11-11

### Added

- *(tui)* Map keyboard layouts to QWERTY bindings on every platform ([44d79e3](https://github.com/chamoretto/silv/commit/44d79e3ffd12450ba6ee6ef40a331a8642915d4a))
- *(tui)* Filter undo/redo keys and chip rows that wrap ([80c2412](https://github.com/chamoretto/silv/commit/80c24129a8da7afc88896838f269ea0d62ff8f13))
- *(tui)* Redo with Ctrl+Y and history keys in the prompt line ([ee7b1f2](https://github.com/chamoretto/silv/commit/ee7b1f23f8d1f9347300ec622a53e7d7902a3904))
- *(tui)* S focuses the field header for keyboard sorting ([1861084](https://github.com/chamoretto/silv/commit/18610848c7830c13e40f4bad0a8c81d7eecd17ae))

### Fixed

- *(tui)* Match key bindings on the physical QWERTY key for non-Latin layouts ([efbdbdf](https://github.com/chamoretto/silv/commit/efbdbdf218cb01fb52b99f1e5f8aa121431e064a))
- *(tui)* Wrap prompt lines and separate them from the rows ([2aa2a36](https://github.com/chamoretto/silv/commit/2aa2a36a8437a984eb1eff5b36bb89a3cd5bdec4))
- *(changelog)* One Unreleased heading and a valid Unreleased link ([2e29472](https://github.com/chamoretto/silv/commit/2e2947293035f3fc072120a2ef4f0184e13f9dfb))
- *(keymap)* Keep macOS layout sources alive while translating ([6ff979a](https://github.com/chamoretto/silv/commit/6ff979a89e5f12e8ae3edc184ed450eb452e9b49))
- *(keymap)* Probe the macOS layout on the main thread ([#5](https://github.com/chamoretto/silv/issues/5)) ([4be7a96](https://github.com/chamoretto/silv/commit/4be7a9678696c63c1212a9a9921efb07a7ed0e11))

### Testing

- *(tui)* Match the prompt line test to the reworded prompts ([7f861eb](https://github.com/chamoretto/silv/commit/7f861eba1dce38e3cc71986768eb00be47b77f20))

### Miscellaneous Tasks

- Add the local three-platform gate script ([ef4b00c](https://github.com/chamoretto/silv/commit/ef4b00ca21bc7c40de462b833f4080cf2601942a))

## [0.1.3] - 2026-11-10

### Fixed

- *(tui)* Key tokens inherit the default foreground on their background ([9442b87](https://github.com/chamoretto/silv/commit/9442b87962ca0a6b5ca99c92b72623b110b3a709))

## [0.1.2] - 2026-11-10

### Added

- *(tui)* Color operators, split chip lines, highlight matches, reorder columns ([1085108](https://github.com/chamoretto/silv/commit/1085108549ac51a9fe1ff3f30254b54755be85c9))
- *(tui)* Prompt line, chip and detail prompts, row value picker ([9c4038c](https://github.com/chamoretto/silv/commit/9c4038cf47388912a53aa333583f8a5a3511df41))
- *(tui)* Visible pick block and key-token prompt highlighting ([5b948ce](https://github.com/chamoretto/silv/commit/5b948ce424af2799d023b8b33b542f186234e71c))

### Fixed

- *(tui)* Space toggles the focused field in the picker ([b16f867](https://github.com/chamoretto/silv/commit/b16f867102bbbf28315fad9272b91074af058dfe))

### Documentation

- Note the PowerShell form of the lock refresh command ([837c861](https://github.com/chamoretto/silv/commit/837c861b8ecc2a5e7ad3448c64794099340c0f00))
- Order the release routine so the cargo gate refreshes the lock ([012427b](https://github.com/chamoretto/silv/commit/012427b1a44d535929902a02bcad7f5aebdde785))

### Miscellaneous Tasks

- *(changelog)* Commit-sha entries and post-commit auto-update hook ([c08ba68](https://github.com/chamoretto/silv/commit/c08ba6805cd5504058087f6a3fb5c8040c794eee))

## [0.1.1] - 2026-11-10

### Fixed

- *(ci)* Hand the generated notes file to the release, not the path string ([386c925](https://github.com/chamoretto/silv/commit/386c9256e04d26e6ecf112a1c1d5f305f5a2078e))

### Documentation

- Deduplicate the 0.1.0 changelog section ([f5ae464](https://github.com/chamoretto/silv/commit/f5ae464a3482ac56420f6710f2ceffbc03463e88))
- Move commit and semver specifications into docs/ ([7991a0b](https://github.com/chamoretto/silv/commit/7991a0b174be9c7e76b3ce8035e39c2fe2405ed7))
- Add interactive TUI screenshot and the maintainer release routine ([3401565](https://github.com/chamoretto/silv/commit/340156583b29a28b6ece28b8a32a491e629bb2b4))

### Miscellaneous Tasks

- Add image binary rules and screenshot asset guide ([3967f54](https://github.com/chamoretto/silv/commit/3967f54e90b0996d413be7ca50b38c5f021df841))
- Emit the unreleased section and version link refs from cliff config ([1e00e44](https://github.com/chamoretto/silv/commit/1e00e4454ba20cfc04992379d305a0d96f3c35c0))

## [0.1.0] - 2026-11-10

### Added

- Implement SILV v0.1.0 Kibana-like log viewer (TUI and grep-like modes) ([f2da592](https://github.com/chamoretto/silv/commit/f2da59263fa5c2294d527042bc580de078eff3cb))
- Guided filter wizard with field autocomplete; fix detail menu Enter ([a715050](https://github.com/chamoretto/silv/commit/a71505065230caeba04f67634a8852371ede3db8))

### Fixed

- Report the correct context for the terminal requirement ([545c697](https://github.com/chamoretto/silv/commit/545c697f4e4199bcbc4dfc5152c63fb84d966cad))
- Stop leaked mouse sequences and hint the search hotkey ([6dda258](https://github.com/chamoretto/silv/commit/6dda25848cdbf8b2b8deaa478a2ce743ce592f9d))
- Swallow ConEmu M-led mouse motion bursts in the key filter ([d9d5b5c](https://github.com/chamoretto/silv/commit/d9d5b5cccf6dc541f4a184b6f89e41ed4ca9ae6a))
- Auto-disable mouse capture on ConEmu-class consoles that leak mouse bytes ([ed760a6](https://github.com/chamoretto/silv/commit/ed760a6106af9f24e1ae4451dacaf38625fe2204))
- Request click and wheel mouse reporting only, drop motion tracking ([0ac7711](https://github.com/chamoretto/silv/commit/0ac771126e55f280b71a82cb763beaa27141da04))
- Auto-disable mouse on JetBrains embedded terminals, dedup filter prompts ([7af5cac](https://github.com/chamoretto/silv/commit/7af5caccd115b6b9223bba65cff98acffb03bf13))

### Documentation

- Add usage guide, key map, filter grammar and static-link matrix ([6888b45](https://github.com/chamoretto/silv/commit/6888b45dee9dc305fb729358f9baf9151779ac97))
- Add changelog via git-cliff ([d5ba17b](https://github.com/chamoretto/silv/commit/d5ba17beb0694b6cd5456114fec9a7f27a70f2f0))
- Rewrite changelog in keep a changelog notation ([55f9c14](https://github.com/chamoretto/silv/commit/55f9c146b9e381cc2abf6fcf75fa025078974f13))
- Add AI disclosure, badges and contributing guide ([4a74bc8](https://github.com/chamoretto/silv/commit/4a74bc898b129e6eba8701092d95de203514bd8f))
- Keep published docs free of internal path references ([e6d87d6](https://github.com/chamoretto/silv/commit/e6d87d624e5c8b8429cc8190060ec75ec44748f5))

### Testing

- Add end-to-end CLI suite (25 cases) and sample log fixtures ([552e7b5](https://github.com/chamoretto/silv/commit/552e7b534294ca52e65d9532899f36f9e61d06f9))

### Miscellaneous Tasks

- Add project scaffolding (gitignore, rustfmt, agents rules, license) ([257ae69](https://github.com/chamoretto/silv/commit/257ae696298f9f18dfc74d23211f4d2f8ed75d1e))
- Add static release build scripts for windows, linux, macos ([3d0af89](https://github.com/chamoretto/silv/commit/3d0af89a1e2a44278cdddc00339f3dc307685090))
- Enforce LF line endings ([99ca532](https://github.com/chamoretto/silv/commit/99ca532f9679a20e8e72492260de3fa44a4983e0))
- Add git-cliff configuration ([dc45b00](https://github.com/chamoretto/silv/commit/dc45b008bb0774f79944db0ec7c400b6f6ba4bda))
- Track Cargo.lock and allow .github in version control ([d6f4d9f](https://github.com/chamoretto/silv/commit/d6f4d9ffcf957b7449523e03f84e215d991bcc37))
- Add CI, release workflow and repository templates ([7bb5301](https://github.com/chamoretto/silv/commit/7bb5301dfbaae96f08ce785c1bf8893ded35a374))
- Map git-cliff groups to keep a changelog notation ([5af60eb](https://github.com/chamoretto/silv/commit/5af60eb20cbf202af8b515cee9e565e57ee0be2e))
- Add repository metadata for github publishing ([4bc36b1](https://github.com/chamoretto/silv/commit/4bc36b1f8b15e17514f4642808da6784febdd1e1))
- Track Cargo.lock for reproducible release builds ([bd3434a](https://github.com/chamoretto/silv/commit/bd3434a420b6adddf60192c2a319d5e9bff9f6c4))


[Unreleased]: https://github.com/chamoretto/silv/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/chamoretto/silv/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/chamoretto/silv/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/chamoretto/silv/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/chamoretto/silv/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/chamoretto/silv/releases/tag/v0.1.0

