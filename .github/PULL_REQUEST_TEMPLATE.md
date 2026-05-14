## Summary

<!-- 1–3 sentences about what changed and why. -->

## Spec / issue references

Closes #<github-issue>
Closes <linear-issue-id>
Spec: <path-to-spec-file-or-N/A>

## Definition of Done

For crate PRs, check all that apply:

- [ ] Public API documented (`#![warn(missing_docs)]` clean)
- [ ] `cargo test` passes
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] Coverage ≥ 90% (or carve-outs documented)
- [ ] Conformance fixtures committed under `conformance/fixtures/<crate>/`
- [ ] Crate README updated (purpose / depends-on / consumers / conformance)
- [ ] CHANGELOG entry added

## Test plan

- [ ] <command + expected outcome>
