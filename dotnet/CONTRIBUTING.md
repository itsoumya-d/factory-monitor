# Contributing

## Setup

```bash
dotnet restore FuelIntegrationHub.slnx
dotnet build FuelIntegrationHub.slnx -c Release
dotnet test FuelIntegrationHub.slnx -c Release
```

## Before opening a pull request

- The change has a test that fails without it.
- `dotnet build FuelIntegrationHub.slnx -c Release` passes with zero warnings.
- `dotnet test FuelIntegrationHub.slnx -c Release` passes.
- `dotnet format FuelIntegrationHub.slnx --verify-no-changes` passes.
- No secrets, customer data, or local database files are staged.
- Auth, HMAC, idempotency, and transaction boundaries are unchanged or strengthened.
- README or docs are updated when an HTTP contract, configuration key, or operational procedure changes.

## Pull request description

Include:

1. The problem and the invariant being protected.
2. The approach and the trade-off.
3. How it was verified (commands and test names).
4. Operational impact, including deployment or rollback notes.

## Review and mentoring

Reviewers should explain the reasoning behind requested changes and link to the relevant test or ADR. Pair on unfamiliar areas. The goal is a codebase that a new engineer can change safely, not a codebase that only its author understands.
