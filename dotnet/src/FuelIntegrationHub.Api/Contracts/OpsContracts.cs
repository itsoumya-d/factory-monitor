namespace FuelIntegrationHub.Api.Contracts;

public sealed record OutboxItemResponse(
    Guid Id,
    string Type,
    string IdempotencyKey,
    string Status,
    int Attempts,
    DateTimeOffset OccurredAt,
    DateTimeOffset NextAttemptAt,
    DateTimeOffset? ProcessedAt,
    string? LastError);

public sealed record OutboxListResponse(
    string? StatusFilter,
    int Count,
    IReadOnlyList<OutboxItemResponse> Items);

public sealed record OutboxRetryResponse(Guid Id, string Status, DateTimeOffset NextAttemptAt);

public sealed record OpsMetricsResponse(
    int Sites,
    int Dispensers,
    int Transactions,
    int TransactionsTodayUtc,
    int OutboxPending,
    int OutboxSent,
    int OutboxDeadLetter,
    int ReconciliationRuns,
    int ReconciliationVariances,
    DateTimeOffset? LastReconciliationAt);

public sealed record SiteResponse(
    Guid Id,
    string Code,
    string Name,
    string Region,
    string Currency,
    bool IsActive,
    int Dispensers);
