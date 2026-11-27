# Repository Guidelines

## Project Structure & Module Organization

Gramwave is a read-only Telegram music player for Linux, built with Rust 2021 and egui. `src/main.rs` launches `src/lib.rs`; `src/bootstrap.rs` assembles the desktop lifecycle.

- `crates/domain/`: models and queue rules; keep free of GUI, database, filesystem, and network dependencies.
- `crates/application/`: commands, ports, reducers, persistence contracts, and task scheduler.
- `src/application/`: desktop presentation controller, control surfaces, and effect execution.
- `src/adapters/`: TDLib, SQLite, audio output, streaming, offline storage, and desktop integrations.
- `src/screens.rs`, `src/ui.rs`, `src/design.rs`: interface and styling.
- `tests/`: desktop integration tests and workspace architecture checks.
- `assets/`: SVG icons, desktop entry, and gettext catalogs. `scripts/` contains translation updates and smoke checks.

Read `ARCHITECTURE.md` before changing layer boundaries or persistence.

## Build, Test, and Development Commands

- `cargo run --no-default-features -- --demo`: preview fictional data without TDLib or account services.
- `cargo run --release --locked`: launch the real player. See [Build and run](README.md#build-and-run) for dependencies and Telegram credentials.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --workspace --locked --all-targets -- -D warnings`: lint; repeat with `--no-default-features` before `--`.
- `cargo test --workspace --locked`: run the default suite; repeat with `--no-default-features`.
- `cargo build --workspace --locked --release`: build an optimized binary.

Use Cargo checks for routine features and commits. Build Gramwave through
Homebrew or run `~/homebrew-gramwave/release.sh` only when the user explicitly
requests a release.

## Documentation

Keep `README.md` focused on what Gramwave does, how to install it and sign in,
and how to run the default local build or demo. Add a detail only if it explains
a distinct user-facing capability or changes what the reader needs to do.
Put build variants and verification in `ARCHITECTURE.md`, and packaging and
release instructions in the [Homebrew tap](https://github.com/mag7ster/homebrew-gramwave).
Link to those instructions from the README.

## Coding Style & Naming Conventions

Use rustfmt defaults, including four-space indentation. Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Route playback actions through shared application commands. Wrap product strings in translation calls; preserve user content and translation placeholders.

## Testing Guidelines

Add or extend an automated test when it catches a meaningful failure that existing tests miss. Prioritize queue order and history, session restoration, SQLite data and migrations, cancellation and stale results, and Telegram's read-only behavior. For a reproducible bug, show that the regression test fails before the fix when practical. There is no numeric coverage target.

Use the narrowest test that can reproduce the failure through the relevant entry point. Keep unit and headless UI tests in a `#[cfg(test)]` module beside the code, using `tests.rs` and topic modules when the suite grows. Put integration tests that exercise package interfaces in that package's `tests/`. Name tests in `snake_case` for the behavior they protect and assert the observable outcome.

Automate UI behavior that matters to users, such as keyboard access, queue interactions, seeking, and controls remaining usable during resize. Check cosmetic changes such as spacing, opacity, and animation motion in the demo. UI changes should include demo screenshots in light and dark themes.

Run native checks with `cargo test --workspace --locked --features native-tests --lib -- --test-threads=1`. These require the platform audio development libraries, TDLib, and loopback sockets. Run `scripts/check-control.py` and `scripts/check-mpris.py` for desktop integration in a graphical D-Bus session. Authenticated playback needs manual verification. See [Verification](ARCHITECTURE.md#verification) for the full command matrix.

## Commit & Pull Request Guidelines

History uses imperative, capitalized subjects such as “Fix…” or “Add…”, without mandatory prefixes. Keep commits focused. PRs should explain behavior, link relevant issues, report validation and manual-test gaps, and include screenshots for visual changes. Run checks for both default and disabled default features.

## Security & Configuration

Never commit credentials, sessions, SQLite databases, or cached media. Account data lives under `$XDG_DATA_HOME/gramwave`. Preserve read-only Telegram behavior and loopback-only streaming.
