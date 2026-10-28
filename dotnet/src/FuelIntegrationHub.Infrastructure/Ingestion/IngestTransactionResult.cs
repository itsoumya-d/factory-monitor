namespace FuelIntegrationHub.Infrastructure.Ingestion;

public sealed record IngestTransactionResult(
    Guid TransactionId,
    string ExternalId,
    string Status,
    decimal Total,
    bool Duplicate);

public sealed record BatchIngestItemResult(
    int Index,
    string ExternalId,
    string Status,
    Guid? TransactionId,
    decimal? Total,
    string? Error);
