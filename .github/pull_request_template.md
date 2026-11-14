## Summary

<!-- What changed and why, in 2–4 plain sentences, as you'd tell someone
     before they open the diff. Link the issue: "Closes #123".
     New feature? Its issue comes first (see CONTRIBUTING.md). -->

## How it works

<!-- The approach in plain words, and any alternative you rejected.
     Then: "Start reading at `path/file.rs` (fn_name)". -->

## Risks

<!-- What could break? "None known" is a fine answer if true.
     Say so if this touches: the encrypted link, pairing, the way-home hotkey,
     grabs or held keys, the clipboard, updates/releases or the installers.
     Those need an agreed issue first: link it. -->

## How it was verified

<!-- Commands you ran and what you saw. For input, clipboard or sound code:
     the real check on two machines (or two terminals), which OS on each side
     (e.g. Windows → Linux), and what happened. -->

## Screenshots

<!-- UI changes only: crop to the Mickey window, nothing personal.
     Delete this section otherwise. -->

## Concept

<!-- Optional. If the change uses a classic data structure or algorithm:
     what it is, the closest LeetCode problem, and why it beats the naive
     approach here. Delete this section otherwise. -->

## Checklist

- [ ] Rebased on the current `main`; the diff shows only my changes (no file reverted)
- [ ] One logical change, about 400 lines of logic or less (lockfiles, translations, generated files and test fixtures don't count), or the Summary says why not
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] New dependency? Justified in the Summary: what it does and why the standard library is not enough
- [ ] AI assistance: none / some / most (I read and understand every line)
- [ ] Docs, or the PRD's *Discoveries*, updated if behaviour changed
