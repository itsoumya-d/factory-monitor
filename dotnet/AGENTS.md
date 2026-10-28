# AGENTS.md

Instructions for AI coding agents working in this repository.

## Build and test loop

Run every command from `dotnet/`:

```bash
dotnet restore FuelIntegrationHub.slnx
dotnet build FuelIntegrationHub.slnx -c Release --no-restore
dotnet test FuelIntegrationHub.slnx -c Release --no-build --no-restore
dotnet format FuelIntegrationHub.slnx --verify-no-changes --no-restore
```

A change is not complete until build, tests, and format checks pass.

## Architecture boundaries

- `FuelIntegrationHub.Domain` holds entities and invariants. It must not reference EF Core, ASP.NET Core, or infrastructure packages.
- `FuelIntegrationHub.Infrastructure` owns persistence, outbox processing, ERP adapters, and application services.
- `FuelIntegrationHub.Api` owns HTTP concerns: authentication, HMAC verification, endpoint contracts, Problem Details, and health checks.
- Endpoints may read through `FuelDbContext` for paged queries. Writes belong in application services so the transaction and outbox boundary stays explicit.
- Do not put business rules in endpoint handlers.

## Non-negotiable rules

- Never commit secrets, connection strings with credentials, API keys, or webhook secrets.
- Never weaken authentication, HMAC verification, idempotency, or validation to make a test pass.
- Every write path that changes a transaction must write its outbox message in the same `SaveChanges` transaction.
- Keep the unique `(site_id, external_id)` index as the idempotency boundary.
- Do not add `.Result`, `.Wait()`, or fire-and-forget async work.
- Do not suppress compiler or analyzer warnings without a written justification.
- Do not add code comments. Use names, types, tests, and documentation instead.
- Keep new dependencies justified. The service deliberately has a small dependency surface.

## Tests

- Add or update a failing test before changing behaviour.
- Domain rules belong in domain tests.
- HTTP contracts, auth, HMAC, persistence, outbox, and reconciliation belong in API integration tests.
- Tests must be deterministic. Do not depend on wall-clock timing; inject `TimeProvider` or use bounded windows.
- Do not call external networks from tests.

## Style

- File-scoped namespaces, nullable enabled, one public type per file.
- Prefer records for immutable contracts and value objects.
- Use `TimeProvider` for time.
- Use structured logging through source-generated `LoggerMessage` delegates.
- Return RFC 9457 Problem Details for HTTP failures.
- Keep public HTTP contracts explicit rather than exposing domain entities.

## AI-assisted change protocol

1. Read the relevant domain, service, endpoint, and test files.
2. Write a short plan with the invariant being protected.
3. Add the failing test.
4. Generate the smallest implementation that makes it pass.
5. Run the full solution test suite, not only the new test.
6. Review the diff for security, idempotency, transaction boundaries, and logging.
7. Update the README or docs when behaviour changes.
