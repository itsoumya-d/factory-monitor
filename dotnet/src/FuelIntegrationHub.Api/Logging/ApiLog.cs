using Microsoft.Extensions.Logging;

namespace FuelIntegrationHub.Api.Logging;

internal static partial class ApiLog
{
    [LoggerMessage(
        EventId = 2001,
        Level = LogLevel.Warning,
        Message = "Rejected webhook with an invalid signature.")]
    public static partial void InvalidWebhookSignature(ILogger logger);

    [LoggerMessage(
        EventId = 2002,
        Level = LogLevel.Error,
        Message = "Unhandled request failure for {Path}")]
    public static partial void RequestFailed(ILogger logger, Exception exception, string path);

    [LoggerMessage(
        EventId = 2003,
        Level = LogLevel.Information,
        Message = "Request rejected for {Path}: {Message}")]
    public static partial void RequestRejected(ILogger logger, string path, string message);
}
