using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using FuelIntegrationHub.Api.Logging;
using Microsoft.AspNetCore.Mvc;
using Microsoft.Extensions.Options;

namespace FuelIntegrationHub.Api.Security;

public sealed class WebhookSignatureMiddleware(
    RequestDelegate next,
    IOptions<SecurityOptions> securityOptions,
    TimeProvider timeProvider,
    ILogger<WebhookSignatureMiddleware> logger)
{
    public const string TimestampHeader = "X-Octane-Timestamp";
    public const string SignatureHeader = "X-Octane-Signature";

    public async Task InvokeAsync(HttpContext context)
    {
        if (!IsIngestRequest(context.Request))
        {
            await next(context);
            return;
        }

        var request = context.Request;
        request.EnableBuffering();

        var timestampValues = request.Headers[TimestampHeader];
        var signatureValues = request.Headers[SignatureHeader];

        if (timestampValues.Count != 1
            || signatureValues.Count != 1
            || !long.TryParse(timestampValues[0], NumberStyles.Integer, CultureInfo.InvariantCulture, out var unixSeconds))
        {
            await WriteUnauthorizedAsync(context);
            return;
        }

        var now = timeProvider.GetUtcNow().ToUnixTimeSeconds();
        if (Math.Abs(now - unixSeconds) > securityOptions.Value.WebhookMaxSkewSeconds)
        {
            await WriteUnauthorizedAsync(context);
            return;
        }

        using var reader = new StreamReader(
            request.Body,
            Encoding.UTF8,
            detectEncodingFromByteOrderMarks: false,
            bufferSize: 1024,
            leaveOpen: true);
        var body = await reader.ReadToEndAsync(context.RequestAborted);
        request.Body.Position = 0;

        var signedPayload = Encoding.UTF8.GetBytes(
            string.Create(CultureInfo.InvariantCulture, $"{unixSeconds}\n{body}"));
        var expected = HMACSHA256.HashData(
            Encoding.UTF8.GetBytes(securityOptions.Value.WebhookSecret),
            signedPayload);

        if (!TryDecodeSignature(signatureValues[0]!, out var provided)
            || !CryptographicOperations.FixedTimeEquals(expected, provided))
        {
            ApiLog.InvalidWebhookSignature(logger);
            await WriteUnauthorizedAsync(context);
            return;
        }

        await next(context);
    }

    private static bool IsIngestRequest(HttpRequest request)
    {
        if (!HttpMethods.IsPost(request.Method))
        {
            return false;
        }

        var segments = request.Path.Value?.Split('/', StringSplitOptions.RemoveEmptyEntries);
        return segments is ["api", "v1", "sites", _, "transactions"]
            or ["api", "v1", "sites", _, "transactions", "batch"];
    }

    private static bool TryDecodeSignature(string signature, out byte[] bytes)
    {
        var value = signature.StartsWith("sha256=", StringComparison.OrdinalIgnoreCase)
            ? signature[7..]
            : signature;

        try
        {
            bytes = Convert.FromHexString(value);
            return bytes.Length == 32;
        }
        catch (FormatException)
        {
            bytes = [];
            return false;
        }
    }

    private static async Task WriteUnauthorizedAsync(HttpContext context)
    {
        context.Response.StatusCode = StatusCodes.Status401Unauthorized;
        await Results.Problem(
            statusCode: StatusCodes.Status401Unauthorized,
            title: "Invalid webhook signature",
            detail: "The request signature or timestamp is missing, expired, or invalid.")
            .ExecuteAsync(context);
    }
}
