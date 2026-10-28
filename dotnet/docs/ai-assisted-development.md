# AI-assisted development

The goal is not to type code faster. The goal is to make the verification loop fast enough that generated code is accountable to tests, constraints, and review.

## Working agreement

1. **The repository is the source of truth.** Prompts and chat transcripts are not design documents. Decisions land in ADRs, contracts, tests, and code.
2. **A failing test precedes a behaviour change.** Agents receive the invariant and the test, not a vague feature request.
3. **Small, bounded diffs.** One concern per change. Generated refactors that touch unrelated files are rejected.
4. **No bypassing guardrails.** AI-generated code cannot weaken auth, HMAC verification, idempotency, validation, logging, or database constraints to make a test green.
5. **Human review is mandatory.** The reviewer checks the transaction boundary, error handling, security, and operational impact.

## Tooling

- GitHub Copilot and Cursor use [`.github/copilot-instructions.md`](../.github/copilot-instructions.md).
- Claude Code and Codex use [`AGENTS.md`](../AGENTS.md).
- The same commands are available to every tool:

```bash
dotnet build FuelIntegrationHub.slnx -c Release
dotnet test FuelIntegrationHub.slnx -c Release
dotnet format FuelIntegrationHub.slnx --verify-no-changes
```

## Prompt pattern

```text
Context: .NET 10 fuel integration service.
Invariant: a transaction and its outbox message are committed atomically.
Change: add a batch endpoint that preserves per-item validation errors.
Constraints: do not change the auth/HMAC pipeline; do not add dependencies; keep endpoints thin.
Proof: add integration tests for mixed valid/invalid batches and duplicate external ids.
Done: build, full test suite, format check pass.
```

## Review checklist for AI-generated changes

- Does the change preserve the transaction-plus-outbox boundary?
- Is the database constraint still the final idempotency guard?
- Are auth and HMAC verification untouched or stronger?
- Are errors mapped to the right status without leaking internals?
- Are logs structured and free of secrets?
- Is there a regression test that fails without the change?
- Did the agent update documentation when a contract changed?

## Adoption plan for a team

1. Start with one repository and one agent tool. Measure review time and escaped defects, not lines generated.
2. Add `AGENTS.md` and tool-specific instruction files before enabling broad agent use.
3. Make the full test and format loop runnable with one command.
4. Require agents to produce the failing test and the verification output.
5. Review prompts and generated diffs in the same way as code.
6. Expand autonomy only after the team trusts the verification loop.
