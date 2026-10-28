namespace FuelIntegrationHub.Infrastructure.Integrations;

public sealed record ErpTransactionEvent(
    string EventId,
    string SiteCode,
    string ExternalId,
    decimal Litres,
    decimal UnitPrice,
    decimal Total,
    DateTimeOffset OccurredAt);
