# Plan
| ID | Task | Owner | Deps |
|---|---|---|---|
| P0.1 | ARCHITECTURE.md | orch (done) | — |
| P0.2 | bench harness (`benches/scan.rs`, synthetic tree gen, frame-time probe) + BASELINE numbers | DeepSeek-1 | — |
| P0.3 | test baseline | orch (done: 4) | — |
| P0.4 | GitHub issues per workstream | orch | — |
| S1 | CI: unsigned dmg/AppImage/deb skeleton; secrets-gated signing steps | DeepSeek-2 | P0 |
| S2a | Size types (Logical/Physical), tree model, hardlink/sparse/unreadable accounting | Astra-6 | P0.2 |
| S2b | Dup-finder hardening + UI no-preselect-all | DeepSeek-3 | S2a |
| S3 | Cancel token, bounded channel, snapshot, cached treemap | Astra-6 | S2a |
| S4 | macOS TCC detection + deep link | DeepSeek | S2a |
| S5 | NsFileManager trash, SECURITY.md, rate-limited deletes, SLSA | Astra-6/DeepSeek | — |
| S6 | tools/fixture-gen + CI | DeepSeek | S2a |
