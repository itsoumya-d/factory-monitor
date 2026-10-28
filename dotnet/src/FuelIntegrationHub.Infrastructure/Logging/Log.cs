using Microsoft.Extensions.Logging;

namespace FuelIntegrationHub.Infrastructure.Logging;

internal static partial class Log
{
    [LoggerMessage(
        EventId = 1001,
        Level = LogLevel.Information,
        Message = "Simulated ERP accepted event {EventId} for site {SiteCode} ({Litres} L)")]
    public static partial void ErpAccepted(
        ILogger logger,
        string eventId,
        string siteCode,
        decimal litres);

    [LoggerMessage(
        EventId = 1002,
        Level = LogLevel.Warning,
        Message = "ERP rejected event {EventId} with status {StatusCode}: {Body}")]
    public static partial void ErpRejected(
        ILogger logger,
        string eventId,
        int statusCode,
        string body);

    [LoggerMessage(
        EventId = 1003,
        Level = LogLevel.Information,
        Message = "Outbox dispatcher is disabled.")]
    public static partial void OutboxDisabled(ILogger logger);

    [LoggerMessage(
        EventId = 1004,
        Level = LogLevel.Error,
        Message = "Outbox dispatcher iteration failed.")]
    public static partial void OutboxIterationFailed(ILogger logger, Exception exception);

    [LoggerMessage(
        EventId = 1005,
        Level = LogLevel.Warning,
        Message = "ERP delivery failed for outbox message {MessageId}; attempt {Attempts}")]
    public static partial void ErpDeliveryFailed(
        ILogger logger,
        Exception exception,
        Guid messageId,
        int attempts);

    [LoggerMessage(
        EventId = 1006,
        Level = LogLevel.Warning,
        Message = "Outbox batch lost a concurrency race; messages will be retried.")]
    public static partial void OutboxConcurrencyRace(ILogger logger, Exception exception);
}
