# Development

Flint targets macOS and Windows. Use Node.js 22, Rust 1.88+, and the platform's native build tools.
See the [README](README.md) and [Windows prerequisites](docs/WINDOWS.md).

```sh
npm ci
./scripts/fetch-sidecars.sh
npm run build
npm test
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Core-only tests run with `cargo test -p convert-core --locked` without bundled sidecars.
See [Verification](docs/VERIFICATION.md) for browser tests, native tests and coverage limits.

## Structure

- `crates/convert-core`: formats, planning, crop, queue, tools and output safety.
- `src-tauri`: native commands, menus, persistence and OS integration.
- `src`: React interface, state and browser preview backend.
- `scripts`: build tools, generators and regression checks.

## Changes

- Preserve input files, validate IPC arguments and keep partial-output cleanup intact.
- Add a regression test for substantive fixes. Do not suppress failures or weaken checks.
- Generate `FORMATS.md` with `npm run gen:formats` and `src/lib/mock-catalog.ts` with
  `npm run gen:catalog`; do not edit generated catalogs by hand.
- Preserve dependency notices. Document licenses for new dependencies.
- Keep setup instructions and reasons for non-obvious constraints; omit development diaries.
- `shots/` is ignored test output. Regenerate tracked screenshots with
  `python3 scripts/shots_readme.py` against the running preview.

The application source is proprietary. Contributions are subject to the owner's agreement;
this file grants no license to the application.
