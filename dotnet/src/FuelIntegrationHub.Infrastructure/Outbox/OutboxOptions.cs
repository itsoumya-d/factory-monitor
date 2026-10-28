namespace FuelIntegrationHub.Infrastructure.Outbox;

public sealed class OutboxOptions
{
    public const string SectionName = "Outbox";

    public bool Enabled { get; init; } = true;

    public int PollIntervalSeconds { get; init; } = 2;

    public int BatchSize { get; init; } = 20;

    public int MaxAttempts { get; init; } = 5;
}
