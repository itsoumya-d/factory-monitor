#!/usr/bin/env bash
set -euo pipefail

BASE="${API_BASE:-http://127.0.0.1:5099}"
API_KEY="${API_KEY:-local-development-api-key-not-a-secret}"
WEBHOOK_SECRET="${WEBHOOK_SECRET:-local-development-webhook-secret-not-a-secret}"
SITE="${SITE:-AKL-01}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for the API smoke test" >&2
  exit 1
fi

EXTERNAL_ID="SMOKE-$(date +%s)-$RANDOM"
OCCURRED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BODY=$(printf '{"externalId":"%s","occurredAt":"%s","dispenserCode":"D1","productCode":"91","litres":40,"unitPrice":2.45,"paymentMethod":"FleetCard"}' "$EXTERNAL_ID" "$OCCURRED_AT")

sign() {
  local timestamp="$1"
  local body="$2"
  printf '%s\n%s' "$timestamp" "$body" \
    | openssl dgst -sha256 -hmac "$WEBHOOK_SECRET" -hex \
    | sed 's/^.* //'
}

post_transaction() {
  local timestamp
  local signature
  timestamp="$(date +%s)"
  signature="$(sign "$timestamp" "$BODY")"

  curl -fsS -X POST "$BASE/api/v1/sites/$SITE/transactions" \
    -H "Content-Type: application/json" \
    -H "X-Api-Key: $API_KEY" \
    -H "X-Octane-Timestamp: $timestamp" \
    -H "X-Octane-Signature: sha256=$signature" \
    -d "$BODY"
}

echo "health"
curl -fsS "$BASE/health"

echo
echo "create"
CREATE_RESPONSE="$(post_transaction)"
echo "$CREATE_RESPONSE" | jq -e --arg id "$EXTERNAL_ID" '.externalId == $id and .status == "recorded"' >/dev/null

echo "idempotent replay"
REPLAY_RESPONSE="$(post_transaction)"
echo "$REPLAY_RESPONSE" | jq -e --arg id "$EXTERNAL_ID" '.externalId == $id and .duplicate == true and .status == "duplicate"' >/dev/null

echo "query"
QUERY_RESPONSE="$(curl -fsS -G "$BASE/api/v1/sites/$SITE/transactions" \
  --data-urlencode "page=1" \
  --data-urlencode "pageSize=50" \
  -H "X-Api-Key: $API_KEY")"
echo "$QUERY_RESPONSE" | jq -e --arg id "$EXTERNAL_ID" '.items | any(.externalId == $id)' >/dev/null

echo "reconciliation"
RECONCILIATION_RESPONSE="$(curl -fsS -X POST "$BASE/api/v1/sites/$SITE/reconciliations" \
  -H "Content-Type: application/json" \
  -H "X-Api-Key: $API_KEY" \
  -d '{"periodStart":"2020-01-01T00:00:00Z","periodEnd":"2030-01-01T00:00:00Z","erpLitres":1,"thresholdPercent":0.5}')"
echo "$RECONCILIATION_RESPONSE" | jq -e '.status == "Balanced" or .status == "VarianceDetected"' >/dev/null

echo "metrics"
METRICS_RESPONSE="$(curl -fsS "$BASE/api/v1/ops/metrics" -H "X-Api-Key: $API_KEY")"
echo "$METRICS_RESPONSE" | jq -e '.transactions >= 1 and .sites >= 1' >/dev/null

echo "smoke: PASS ($EXTERNAL_ID)"
