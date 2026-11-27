# AGENTS.md

Instructions for AI coding agents working in this repository. `CLAUDE.md` is a
symlink to this file.

## Pull Requests

- Open completed pull requests as ready for review and enable auto-merge,
  so required CI controls merging. Do not bypass checks or push directly
  to `main`.
- Title pull requests as conventional commits,
  `<type>[(scope)][!]: <description>` (the `pr-title` check enforces it).
  Add `!` when operators must act on the change (removed or renamed flag,
  config key or RPC field, on-disk format, consensus-breaking): daily
  releases bump their version from these titles (`docs/releasing.md`).
- Do not request human reviews. Pull requests in this repository need no
  human approval; they merge when required CI passes.
- Do not include implementation plans (step-by-step task lists, agent
  execution plans, `docs/**/plans/` files) in PRs. Keep plans in your session;
  commit only the code, tests, and lasting docs such as specs and ADRs.
- A pull request that changes `.github/workflows/ci.yml` also refreshes its
  SHA-256 in `conformance/mainnet-rpc/readiness.json` (`bindings`), and its
  description says what changed in CI. Check with
  `python3 conformance/mainnet-rpc/readiness.py`. Agents may do this with
  auto-merge enabled; it needs no owner approval. Without the refresh the
  `rpc-evidence` check fails on `main` for every pull request.
