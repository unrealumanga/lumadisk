# Profiles (session 2026-10-22; no observations yet)
## DeepSeek (stand-in: Claude subagent, sonnet) — volume coder; cheap. Reliability: unrated.
## Astra-6 (stand-in: Claude subagent, opus) — arch/security/integration; premium. Reliability: unrated.
## NEDSTER — ≤50-line tasks; see nedster-companion-profile memory (vague cleanup deletes; false premises).
## opencode / codex / Gemini — mechanical / parallel / second opinion. Unrated.
- DeepSeek-1 (sonnet subagent), P0.2: delivered measured numbers, caveats stated honestly, no new deps; but branched from main not my branch (merge conflicts) and omitted cold-cache. Reliability: high so far.
- DeepSeek (W-03): precise, cited source lines (verified by orchestrator), honest about untested macOS path. Reliability: high.
- DeepSeek (W-04): thorough, honest UNVERIFIED list; but left an unpinned download (orchestrator pinned sha256). Check supply-chain pins on review.
- Astra-6 (W-02): high. Caught a wrong API in the brief by measuring, stated bench delta honestly, listed gaps + UNVERIFIED. Did not launch the GUI (UI unseen).
- DeepSeek (W-07): high; flagged its own risk (webbrowser may not deliver the x-apple scheme). Claim 'NSWorkspace, no process' verified.
- DeepSeek (W-08): high; assertions from generator manifest (verified). 'No bugs found' plausible post-W-02 but physical total only upper-bounded; cases that cannot be built print SKIPPED and pass (0 skipped on this box).
