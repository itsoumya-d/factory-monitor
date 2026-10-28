# ADR 0001: Modular monolith for the integration slice

- Status: Accepted
- Date: 2026-10-28

## Context

The integration slice has three distinct concerns: HTTP edge, domain invariants, and persistence/ERP delivery. Splitting them into independently deployed services would add network failure modes, distributed transactions, and operational overhead before the domain or traffic profile justifies it.

## Decision

Use a modular monolith with three projects:

- `FuelIntegrationHub.Domain` for invariants;
- `FuelIntegrationHub.Infrastructure` for persistence, adapters, and application services;
- `FuelIntegrationHub.Api` for HTTP concerns.

The projects enforce dependency direction at compile time. The API and outbox worker deploy together, but the outbox table provides a durable boundary that would survive a later split.

## Consequences

Positive:

- One deployment, one database transaction for the transaction-plus-outbox write.
- Fast local test loop and simple debugging.
- Clear seams for a future extraction of the ERP delivery worker.

Negative:

- The API and worker scale together in the current deployment.
- The database is a shared dependency; SQLite limits horizontal scale.
- Discipline is required to keep endpoint handlers thin and the domain free of framework references.

## Review trigger

Revisit when ERP delivery volume, independent scaling needs, or multi-tenant isolation justify a separate worker or service boundary.
