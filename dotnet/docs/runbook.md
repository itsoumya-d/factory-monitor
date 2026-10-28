# Runbook

## Health

```bash
curl -fsS https://<api>/health
curl -fsS https://<api>/health/ready
```

Liveness checks the process. Readiness checks the database connection. A failed readiness check means the API should not receive traffic.

## Outbox backlog

```bash
curl -fsS -H "X-Api-Key: $API_KEY" "https://<api>/api/v1/ops/outbox?status=Pending&limit=200"
curl -fsS -H "X-Api-Key: $API_KEY" "https://<api>/api/v1/ops/metrics"
```

If pending count grows and age exceeds the delivery SLO:

1. Check ERP availability and credentials.
2. Inspect `lastError` on the oldest pending messages.
3. Confirm the dispatcher is running and `Outbox__Enabled=true`.
4. Fix the upstream issue, then let the backoff retry.
5. If the backlog is too old for the delivery SLO, coordinate with finance before replaying.

## Dead-letter messages

```bash
curl -fsS -H "X-Api-Key: $API_KEY" "https://<api>/api/v1/ops/outbox?status=DeadLetter&limit=200"
```

For each dead letter:

1. Read `lastError` and confirm the ERP rejected the event for a transient or permanent reason.
2. Confirm the ERP has not already accepted the idempotency key.
3. Correct the payload or ERP configuration if needed.
4. Requeue with `POST /api/v1/ops/outbox/{id}/retry`.
5. Verify the message moves to `Sent`.

Do not delete dead letters. Delete only after finance confirms the event is either accepted or intentionally abandoned, and retain an audit record.

## Reconciliation variance

```bash
curl -fsS -H "X-Api-Key: $API_KEY" \
  "https://<api>/api/v1/sites/AKL-01/reconciliations?limit=20"
```

A `VarianceDetected` run is not automatically an error. It means site litres and ERP litres differ beyond the requested threshold. Compare:

- the transaction window and site timezone;
- voided transactions;
- ERP posting cut-off;
- duplicate or missing external ids;
- unit conversion.

Record the investigation outcome before closing the incident.

## Database backup

SQLite on Azure Files:

1. Stop writes or put the API into maintenance.
2. Copy the `.db` file and verify integrity with `sqlite3 <file> "PRAGMA integrity_check;"`.
3. Store the backup outside the container share.
4. Restore to a staging instance before production.

For Azure SQL/PostgreSQL, use point-in-time restore and test the restore procedure quarterly.

## Credential rotation

1. Generate a new API key and/or webhook secret.
2. Update the platform secret store.
3. Roll the container revision.
4. Update integration clients.
5. Remove the old credential after the overlap window.

Never rotate by committing a value to the repository.
