namespace FuelIntegrationHub.Api.Contracts;

public sealed record IngestTransactionRequest(
    string ExternalId,
    DateTimeOffset OccurredAt,
    string DispenserCode,
    string ProductCode,
    decimal Litres,
    decimal UnitPrice,
    string PaymentMethod);

public sealed record IngestTransactionResponse(
    Guid TransactionId,
    string ExternalId,
    string Status,
    decimal Total,
    bool Duplicate);

public sealed record BatchIngestRequest(IReadOnlyList<IngestTransactionRequest> Transactions);

public sealed record BatchIngestResponse(IReadOnlyList<BatchIngestItemResponse> Results);

public sealed record BatchIngestItemResponse(
    int Index,
    string ExternalId,
    string Status,
    Guid? TransactionId,
    decimal? Total,
    string? Error);

public sealed record TransactionQueryItem(
    Guid TransactionId,
    string ExternalId,
    DateTimeOffset OccurredAt,
    string DispenserCode,
    string ProductCode,
    decimal Litres,
    decimal UnitPrice,
    decimal Total,
    string PaymentMethod,
    string Status);

public sealed record TransactionQueryResponse(
    string SiteCode,
    DateTimeOffset From,
    DateTimeOffset To,
    int Page,
    int PageSize,
    int TotalCount,
    IReadOnlyList<TransactionQueryItem> Items);
