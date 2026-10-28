using FuelIntegrationHub.Domain.Transactions;

namespace FuelIntegrationHub.Infrastructure.Ingestion;

public sealed record IngestTransactionCommand(
    string ExternalId,
    DateTimeOffset OccurredAt,
    string DispenserCode,
    string ProductCode,
    decimal Litres,
    decimal UnitPrice,
    PaymentMethod PaymentMethod);
