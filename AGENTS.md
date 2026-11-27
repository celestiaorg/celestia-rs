# AGENTS.md

Instructions for AI coding agents working in this repository. `CLAUDE.md` is a
symlink to this file.

## Pull Requests

- Open completed pull requests as ready for review and enable auto-merge,
  so required CI controls merging. Do not bypass checks or push directly
  to `main`.
- Do not request human reviews. Pull requests in this repository need no
  human approval; they merge when required CI passes.
- Do not include implementation plans (step-by-step task lists, agent
  execution plans, `docs/**/plans/` files) in PRs. Keep plans in your session;
  commit only the code, tests, and lasting docs such as specs and ADRs.
- A pull request that changes `.github/workflows/ci.yml` says in its
  description what changed in CI.
- Do not add a content hash that pins a file ordinary pull requests edit
  (documentation, source, workflows). Hashes are for frozen evidence such as
  captured logs and manifests. `conformance/mainnet-rpc/readiness.json` and
  `scope-audit.json` follow this rule.
