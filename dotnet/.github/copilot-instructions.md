# GitHub Copilot instructions

This is a .NET 10 integration service for fuel-site transactions and ERP delivery.

## Commands

```bash
dotnet build FuelIntegrationHub.slnx -c Release
dotnet test FuelIntegrationHub.slnx -c Release
dotnet format FuelIntegrationHub.slnx --verify-no-changes
```

## Context

- Domain invariants live in `src/FuelIntegrationHub.Domain`.
- Persistence and application services live in `src/FuelIntegrationHub.Infrastructure`.
- HTTP, auth, HMAC, and contracts live in `src/FuelIntegrationHub.Api`.
- `FuelIntegrationHub.Domain` must never depend on EF Core or ASP.NET Core.
- Transaction writes and their outbox message must be committed by one `SaveChanges`.
- The unique `(site_id, external_id)` index is the idempotency guarantee.
- All `/api` routes require `X-Api-Key`; transaction POSTs also require HMAC-SHA256.
- Use `TimeProvider`, not `DateTimeOffset.UtcNow`, in production code.
- Use source-generated `LoggerMessage` delegates, not interpolated log strings.
- Never weaken auth, signature validation, idempotency, or constraints to satisfy a test.
- Never add secrets or real customer data.
- Do not add comments to code. Improve names and tests instead.

## Definition of done

Build and tests pass, format check passes, new behaviour has a test, HTTP errors are Problem Details, and the README/docs reflect any contract change.
