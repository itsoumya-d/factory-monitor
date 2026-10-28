namespace FuelIntegrationHub.Api.Contracts;

public sealed record ReconciliationRequest(
    DateTimeOffset PeriodStart,
    DateTimeOffset PeriodEnd,
    decimal ErpLitres,
    decimal ThresholdPercent = 0.5m);

public sealed record ReconciliationResponse(
    Guid ReconciliationId,
    string SiteCode,
    DateTimeOffset PeriodStart,
    DateTimeOffset PeriodEnd,
    decimal SiteLitres,
    decimal ErpLitres,
    decimal VarianceLitres,
    decimal VariancePercent,
    decimal ThresholdPercent,
    string Status,
    DateTimeOffset CreatedAt);
