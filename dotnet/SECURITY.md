# Security

## Supported versions

Security fixes target the current `main` branch and the latest .NET 10 patch release.

## Reporting

Report a vulnerability privately to the repository owner. Do not open a public issue with exploit details or customer data. Include the affected endpoint, a reproduction, and the impact you believe is possible.

## Threat model

The service is an integration boundary between untrusted site/POS traffic and an ERP. The highest-risk paths are:

| Threat | Control |
|---|---|
| Forged transaction webhook | HMAC-SHA256 over timestamp and raw body; constant-time comparison; clock-skew limit |
| Replay of a valid webhook | Timestamp window; unique `(site_id, external_id)` idempotency boundary |
| Unauthorised API access | API key required on every `/api` route; constant-time comparison |
| Duplicate ERP delivery | Transactional outbox; stable idempotency key; at-least-once contract |
| Partial write between transaction and outbox | Both records are committed in one database transaction |
| SQL injection | Parameterised EF Core queries; no string-built SQL from request data |
| Secret leakage | Secrets come from configuration/environment; no secrets in source or logs |
| Error-detail leakage | Problem Details; stack traces are logged, not returned |
| Dependency compromise | Minimal dependency surface; GitHub Actions vulnerability audit; Dependabot |

## Operational requirements

- Use a unique, high-entropy API key per integration client. The API key is a shared credential in this reference implementation; production deployments should move to per-client credentials and rotation.
- Rotate `Security:ApiKey` and `Security:WebhookSecret` through the deployment platform, not by editing committed configuration.
- Keep the HMAC clock-skew window as small as operational constraints allow.
- Terminate TLS at the ingress and do not expose the container port directly.
- Restrict outbox and reconciliation endpoints to operations identities. The reference implementation uses the same integration role for simplicity; split roles before exposing the ops API to a wider audience.
- Back up the SQLite file on Azure Files, or move to Azure SQL/PostgreSQL with point-in-time restore before production use.
- Monitor dead-letter counts and reconciliation variance alerts.

## Data handling

Fuel transactions contain commercial data, not payment-card data. The service does not store card numbers or customer identities. Do not add personal data to outbox payloads without a documented retention and access policy.
