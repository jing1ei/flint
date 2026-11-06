<!-- Keep this short. Delete anything that does not apply. -->

**What this changes, and why**

**Fixes #**

**How you know it works** — for a bug fix, name the regression test you added and confirm it fails
without the fix.

- [ ] `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] `cargo fmt --all --check` and `npm run build` pass
- [ ] `python3 scripts/ui_behaviour.py` and `python3 scripts/deadcss.py` pass (UI changes)
- [ ] Generated files regenerated, not hand-edited (`npm run gen:formats`, `npm run gen:catalog`)
- [ ] New dependencies, if any, have their licence noted in `THIRD-PARTY-LICENSES.md`
