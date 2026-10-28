# ADR 0002: Transactional outbox for ERP delivery

- Status: Accepted
- Date: 2026-10-28

## Context

Writing a fuel transaction and calling the ERP in the same request creates a dual-write problem. If the ERP call fails after the database commit, the transaction is invisible to finance. If the ERP call succeeds before the database commit fails, the ERP has a transaction the site system will retry.

## Decision

Persist an outbox message in the same EF Core `SaveChanges` transaction as the fuel transaction. A background dispatcher delivers pending messages with bounded exponential backoff. Each message carries a stable idempotency key. After the configured maximum attempts the message becomes a dead letter and requires an explicit ops requeue.

## Consequences

Positive:

- No lost transactions after a successful API response.
- Retries are durable and observable.
- ERP failure does not block the ingest request.
- Dead letters are inspectable and replayable.

Negative:

- Delivery is at-least-once; the ERP must deduplicate by idempotency key.
- The outbox table grows and needs retention/archival.
- A stuck dispatcher delays ERP visibility even though the API remains available.

## Operational requirements

- Alert on dead-letter count and on outbox age above the delivery SLO.
- Keep the ERP idempotency contract versioned with the event payload.
- Archive sent messages on a schedule once the finance retention window is agreed.
