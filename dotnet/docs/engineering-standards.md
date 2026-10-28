# Engineering standards

## Design

- Model invariants in the domain, not in controllers or SQL triggers alone.
- Keep endpoint handlers thin: validate transport shape, call an application service, map the result.
- Use database constraints for invariants that must survive concurrent writers.
- Make idempotency a database property, not only an `if` statement.
- Prefer explicit contracts over exposing entities.
- Write ADRs for decisions that are expensive to reverse.

## Code review

A reviewer should be able to answer:

1. What invariant or behaviour does this change protect?
2. What test would fail if the change were removed?
3. Does the write path preserve the transaction-plus-outbox boundary?
4. Are auth, HMAC, idempotency, and error handling unchanged or strengthened?
5. Are new dependencies justified?
6. Is there a migration, rollback, or operational note?

Review comments should be specific and actionable. Use pairing for unfamiliar areas and for first-time contributors.

## Testing

- Domain tests are fast and framework-free.
- Integration tests exercise the real HTTP pipeline, auth, HMAC, EF Core, and persistence.
- A bug fix starts with a regression test.
- Tests must not depend on wall-clock timing, external networks, or shared mutable state across classes.
- Coverage is a signal, not a target. A critical invariant with one focused test is worth more than broad shallow coverage.

## Observability

- Structured logs only; no sensitive payloads.
- Log the event that matters, not every line of control flow.
- Health checks distinguish liveness from readiness.
- Outbox and reconciliation metrics are operational signals, not vanity metrics.
- Every alert should link to a runbook action.

## Security

- Treat all inbound data as untrusted.
- Verify signatures over raw bytes.
- Use constant-time comparisons for secrets.
- Return Problem Details without stack traces.
- Keep secrets out of source, logs, and error responses.
- Rotate credentials through the platform.

## Mentoring

- Pair on the first change in an unfamiliar area.
- Ask for the smallest test that proves the behaviour.
- Explain trade-offs with evidence, not preference.
- Leave the codebase easier to change than you found it.
