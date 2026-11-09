# Repository Guidelines

## Project Structure & Module Organization

Gramwave is a read-only Telegram music player for Linux, built with Rust 2021 and egui. `src/main.rs` launches `src/lib.rs`; `src/bootstrap.rs` assembles the desktop lifecycle.

- `crates/domain/`: models and queue rules; keep free of GUI, database, filesystem, and network dependencies.
- `crates/application/`: commands, ports, reducers, persistence contracts, and task scheduler.
- `src/application/`: desktop presentation controller, control surfaces, and effect execution.
- `src/adapters/`: TDLib, SQLite, mpv, streaming, offline storage, and desktop integrations.
- `src/screens.rs`, `src/ui.rs`, `src/design.rs`: interface and styling.
- `tests/`: integration tests; unit tests usually live beside modules in `tests.rs`.
- `assets/`: SVG icons, desktop entry, and gettext catalogs. `scripts/` contains packaging and smoke checks.

Read `ARCHITECTURE.md` before changing layer boundaries or persistence.

## Build, Test, and Development Commands

- `cargo run --no-default-features -- --demo`: preview fictional data without TDLib or account services.
- `cargo run --release`: launch the real player; requires libmpv and TDLib runtime dependencies listed in `README.md`.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --workspace --locked --all-targets -- -D warnings`: lint; repeat with `--no-default-features` before `--`.
- `cargo test --workspace --locked`: run the default suite; repeat with `--no-default-features`.
- `cargo build --workspace --locked --release`: build an optimized binary.
- `scripts/package.sh`: create the local bundle in `dist/tg-player`.

## Coding Style & Naming Conventions

Use rustfmt defaults, including four-space indentation. Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Route playback actions through shared application commands. Wrap product strings in translation calls; preserve user content and translation placeholders.

## Testing Guidelines

Use Rust's built-in `#[test]` framework with descriptive `snake_case` names. Default tests cover queue behavior, headless UI, parsing, and temporary SQLite stores. Add regression coverage for behavioral changes; no numeric coverage threshold is configured.

Run native checks with `cargo test --workspace --locked --features native-tests --lib -- --test-threads=1`; these require libmpv, TDLib, and loopback sockets. UI changes should include demo screenshots in light/dark themes. Authenticated playback needs manual verification.

## Commit & Pull Request Guidelines

History uses imperative, capitalized subjects such as “Fix…” or “Add…”, without mandatory prefixes. Keep commits focused. PRs should explain behavior, link relevant issues, report validation and manual-test gaps, and include screenshots for visual changes. Run checks for both default and disabled default features.

## Security & Configuration

Never commit credentials, sessions, SQLite databases, or cached media. Account data lives under `$XDG_DATA_HOME/tg-player`. Preserve read-only Telegram behavior and loopback-only streaming.
