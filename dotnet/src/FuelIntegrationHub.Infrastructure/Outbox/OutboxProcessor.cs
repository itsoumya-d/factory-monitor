using System.Text.Json;
using FuelIntegrationHub.Domain.Outbox;
using FuelIntegrationHub.Infrastructure.Integrations;
using FuelIntegrationHub.Infrastructure.Logging;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace FuelIntegrationHub.Infrastructure.Outbox;

public sealed class OutboxProcessor(
    FuelDbContext db,
    IErpClient erpClient,
    TimeProvider timeProvider,
    IOptions<OutboxOptions> options,
    ILogger<OutboxProcessor> logger)
{
    public async Task<int> ProcessBatchAsync(CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow();
        var messages = await db.OutboxMessages
            .Where(message =>
                message.Status == OutboxStatus.Pending
                && message.NextAttemptAt <= now)
            .OrderBy(message => message.OccurredAt)
            .Take(Math.Max(1, options.Value.BatchSize))
            .ToListAsync(cancellationToken);

        if (messages.Count == 0)
        {
            return 0;
        }

        foreach (var message in messages)
        {
            message.MarkProcessing();

            try
            {
                var transactionEvent = JsonSerializer.Deserialize<ErpTransactionEvent>(message.Payload)
                    ?? throw new InvalidOperationException("Outbox payload could not be deserialized.");

                await erpClient.SendTransactionAsync(transactionEvent, cancellationToken);
                message.MarkSent(timeProvider.GetUtcNow());
            }
            catch (Exception exception) when (exception is not OperationCanceledException)
            {
                Log.ErpDeliveryFailed(logger, exception, message.Id, message.Attempts + 1);

                message.MarkFailed(exception.Message, timeProvider.GetUtcNow(), options.Value.MaxAttempts);
            }
        }

        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateConcurrencyException exception)
        {
            Log.OutboxConcurrencyRace(logger, exception);
        }

        return messages.Count;
    }
}
