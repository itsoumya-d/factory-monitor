using FuelIntegrationHub.Api.Logging;
using FuelIntegrationHub.Domain.Common;
using Microsoft.AspNetCore.Diagnostics;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Api.ErrorHandling;

public sealed class GlobalExceptionHandler(ILogger<GlobalExceptionHandler> logger) : IExceptionHandler
{
    public async ValueTask<bool> TryHandleAsync(
        HttpContext httpContext,
        Exception exception,
        CancellationToken cancellationToken)
    {
        var path = httpContext.Request.Path.Value ?? "/";

        var (statusCode, title) = exception switch
        {
            DomainException => (StatusCodes.Status400BadRequest, "Domain validation failed"),
            KeyNotFoundException => (StatusCodes.Status404NotFound, "Resource not found"),
            DbUpdateException => (StatusCodes.Status409Conflict, "Data conflict"),
            _ => (StatusCodes.Status500InternalServerError, "Unexpected server error")
        };

        if (statusCode >= 500)
        {
            if (logger.IsEnabled(LogLevel.Error))
            {
                ApiLog.RequestFailed(logger, exception, path);
            }
        }
        else if (logger.IsEnabled(LogLevel.Information))
        {
            ApiLog.RequestRejected(logger, path, exception.Message);
        }

        httpContext.Response.StatusCode = statusCode;
        await httpContext.Response.WriteAsJsonAsync(
            new ProblemDetails
            {
                Status = statusCode,
                Title = title,
                Detail = statusCode >= 500 ? "An unexpected error occurred." : exception.Message,
                Instance = path
            },
            cancellationToken);

        return true;
    }
}
