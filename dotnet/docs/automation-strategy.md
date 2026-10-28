# Automation and integration assurance strategy

## Objective

Make integration behaviour verifiable before, during, and after deployment. The goal is not a large test count. The goal is fast feedback on the contracts, failure modes, and operational guarantees that decide whether an integration is safe to release.

## Test pyramid

| Layer | What it proves | Tooling here |
|---|---|---|
| Domain unit tests | Invariants, rounding, state transitions, retry policy | xUnit, no framework dependencies |
| API integration tests | Auth, HMAC, idempotency, persistence, outbox, reconciliation, health | `WebApplicationFactory`, real HTTP pipeline, SQLite in-memory, real EF Core |
| End-to-end smoke | The built API starts, accepts a signed request, replays it safely, queries it, reconciles it, and exposes metrics | `scripts/verify-api.sh`, curl, jq, openssl |
| Performance benchmark | The write path and aggregate queries remain inside the expected envelope | `bench/FuelIntegrationHub.Benchmarks` |
| Production assurance | Health, readiness, dead-letter counts, reconciliation variance, outbox age | `/health`, `/health/ready`, `/api/v1/ops/metrics`, `/api/v1/ops/outbox` |

## CI quality gates

Every change under `dotnet/` must pass:

1. `dotnet restore`
2. `dotnet format --verify-no-changes`
3. Release build with warnings as errors
4. Full test suite with coverage collection
5. Transitive NuGet vulnerability audit
6. API smoke test against the published build
7. Publish artifact

The smoke test is intentionally separate from the unit and API integration suites. It catches startup, configuration, routing, and wiring failures that component tests can miss.

## Integration-specific controls

- Sign the raw request body, not a reserialized object.
- Verify the signature with a constant-time comparison and a bounded clock skew.
- Reject missing or malformed credentials before the endpoint executes.
- Use a database unique index as the idempotency boundary.
- Commit the transaction and outbox message in one transaction.
- Retry ERP delivery with bounded exponential backoff.
- Dead-letter after the configured maximum attempts and require an explicit requeue.
- Reconcile aggregated site litres against ERP litres with an explicit threshold.
- Expose health, readiness, outbox, and metrics signals for operations.

## Test data and determinism

- API integration tests use a shared in-memory SQLite connection and unique external ids.
- Time-dependent tests use bounded windows or inject `TimeProvider`; they do not sleep.
- The smoke test uses a unique external id and asserts idempotency by replaying the same request.
- No test depends on an external network or a live ERP.

## Flaky-test policy

A flaky test is a production signal, not a nuisance. Quarantine it only with an owner, a linked issue, and a deadline. Fix the root cause: shared state, wall-clock dependence, port contention, or nondeterministic ordering. Do not hide it with retries.

## Extending the framework

The next integration surfaces worth adding:

- Consumer-driven contract tests for ERP and third-party APIs.
- Postman/Newman collections for partner-facing API verification.
- MuleSoft/Boomi connector smoke tests where those platforms are part of the integration estate.
- Testcontainers-backed PostgreSQL/SQL Server runs for provider-specific SQL.
- k6 or NBomber load tests for ingest and reconciliation throughput.
- Playwright checks for the operations dashboard when a UI exists.
- Mutation testing on domain invariants to measure test quality, not just coverage.

## AI-assisted automation

AI tools are useful for generating candidate test cases, edge-case matrices, and failure-mode checklists. They are not trusted to decide whether a release is safe. Generated tests are reviewed like production code, run in CI, and must fail for the right reason before they are accepted. The `AGENTS.md` and Copilot instructions keep the same guardrails in scope for coding agents.
