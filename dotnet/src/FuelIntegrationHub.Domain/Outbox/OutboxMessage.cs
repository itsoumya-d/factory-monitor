using FuelIntegrationHub.Domain.Common;

namespace FuelIntegrationHub.Domain.Outbox;

public sealed class OutboxMessage
{
    private OutboxMessage()
    {
    }

    public Guid Id { get; private set; }

    public string Type { get; private set; } = string.Empty;

    public string Payload { get; private set; } = string.Empty;

    public string IdempotencyKey { get; private set; } = string.Empty;

    public OutboxStatus Status { get; private set; }

    public int Attempts { get; private set; }

    public DateTimeOffset OccurredAt { get; private set; }

    public DateTimeOffset NextAttemptAt { get; private set; }

    public DateTimeOffset? ProcessedAt { get; private set; }

    public string? LastError { get; private set; }

    public static OutboxMessage Enqueue(string type, string payload, string idempotencyKey, DateTimeOffset now)
    {
        var timestamp = now.ToUniversalTime();

        return new OutboxMessage
        {
            Id = Guid.NewGuid(),
            Type = Guard.NotBlank(type, nameof(type), 120),
            Payload = Guard.NotBlank(payload, nameof(payload), 65_536),
            IdempotencyKey = Guard.NotBlank(idempotencyKey, nameof(idempotencyKey), 200),
            Status = OutboxStatus.Pending,
            OccurredAt = timestamp,
            NextAttemptAt = timestamp
        };
    }

    public void MarkProcessing()
    {
        if (Status is not (OutboxStatus.Pending or OutboxStatus.Processing))
        {
            throw new DomainException("Only pending or processing messages can be claimed.");
        }

        Status = OutboxStatus.Processing;
    }

    public void MarkSent(DateTimeOffset now)
    {
        Status = OutboxStatus.Sent;
        ProcessedAt = now.ToUniversalTime();
        LastError = null;
    }

    public void MarkFailed(string error, DateTimeOffset now, int maxAttempts)
    {
        Attempts++;
        LastError = error.Length > 2_000 ? error[..2_000] : error;
        ProcessedAt = null;

        if (Attempts >= maxAttempts)
        {
            Status = OutboxStatus.DeadLetter;
            return;
        }

        Status = OutboxStatus.Pending;
        NextAttemptAt = now.ToUniversalTime().Add(OutboxBackoff.NextDelay(Attempts));
    }

    public void Requeue(DateTimeOffset now)
    {
        Status = OutboxStatus.Pending;
        Attempts = 0;
        LastError = null;
        ProcessedAt = null;
        NextAttemptAt = now.ToUniversalTime();
    }
}
