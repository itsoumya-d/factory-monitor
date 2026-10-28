using System.Globalization;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using FuelIntegrationHub.Api.Contracts;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;

namespace FuelIntegrationHub.Api.Tests;

public sealed class ApiFactory : WebApplicationFactory<Program>
{
    public const string ApiKey = "test-value-000000";
    public const string WebhookSecret = "test-value-111111";

    private readonly SqliteConnection _connection = new("Data Source=:memory:");

    public static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    public ApiFactory()
    {
        _connection.Open();
    }

    protected override void ConfigureWebHost(IWebHostBuilder builder)
    {
        builder.UseEnvironment("Testing");
        builder.ConfigureAppConfiguration((_, configuration) =>
        {
            configuration.AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["ConnectionStrings:Default"] = "Data Source=:memory:",
                ["Database:EnsureCreated"] = "true",
                ["Database:Seed"] = "true",
                ["Database:DetailedErrors"] = "true",
                ["Security:ApiKey"] = ApiKey,
                ["Security:WebhookSecret"] = WebhookSecret,
                ["Security:WebhookMaxSkewSeconds"] = "300",
                ["Outbox:Enabled"] = "false",
                ["Outbox:BatchSize"] = "50",
                ["Outbox:MaxAttempts"] = "3",
                ["Erp:Mode"] = "Simulated",
                ["Logging:LogLevel:Default"] = "Warning"
            });
        });

        builder.ConfigureServices(services =>
        {
            services.RemoveAll<DbContextOptions<FuelDbContext>>();
            services.RemoveAll<FuelDbContext>();
            services.AddDbContext<FuelDbContext>(options => options.UseSqlite(_connection));
        });
    }

    public HttpClient CreateAuthenticatedClient()
    {
        var client = CreateClient();
        client.DefaultRequestHeaders.Add("X-Api-Key", ApiKey);
        return client;
    }

    public static async Task<HttpResponseMessage> PostTransactionAsync(
        HttpClient client,
        string siteCode,
        IngestTransactionRequest request,
        bool sign = true)
    {
        var body = JsonSerializer.Serialize(request, JsonOptions);
        return await SendSignedAsync(
            client,
            HttpMethod.Post,
            $"/api/v1/sites/{siteCode}/transactions",
            body,
            sign);
    }

    public static async Task<HttpResponseMessage> PostBatchAsync(
        HttpClient client,
        string siteCode,
        BatchIngestRequest request)
    {
        var body = JsonSerializer.Serialize(request, JsonOptions);
        return await SendSignedAsync(
            client,
            HttpMethod.Post,
            $"/api/v1/sites/{siteCode}/transactions/batch",
            body,
            sign: true);
    }

    public static async Task<HttpResponseMessage> SendSignedAsync(
        HttpClient client,
        HttpMethod method,
        string path,
        string body,
        bool sign = true)
    {
        var timestamp = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var request = new HttpRequestMessage(method, path)
        {
            Content = new StringContent(body, Encoding.UTF8, "application/json")
        };

        if (sign)
        {
            request.Headers.Add("X-Octane-Timestamp", timestamp.ToString(CultureInfo.InvariantCulture));
            request.Headers.Add("X-Octane-Signature", "sha256=" + Sign(body, timestamp));
        }

        return await client.SendAsync(request);
    }

    public static string Sign(string body, long unixSeconds)
    {
        var payload = Encoding.UTF8.GetBytes(
            string.Create(CultureInfo.InvariantCulture, $"{unixSeconds}\n{body}"));
        var signature = HMACSHA256.HashData(Encoding.UTF8.GetBytes(WebhookSecret), payload);
        return Convert.ToHexString(signature).ToLowerInvariant();
    }

    public static IngestTransactionRequest CreateRequest(
        string? externalId = null,
        decimal litres = 40m,
        decimal unitPrice = 2.45m,
        string dispenserCode = "D1",
        string productCode = "91",
        DateTimeOffset? occurredAt = null)
    {
        return new IngestTransactionRequest(
            externalId ?? Guid.NewGuid().ToString("N"),
            occurredAt ?? DateTimeOffset.UtcNow.AddMinutes(-5),
            dispenserCode,
            productCode,
            litres,
            unitPrice,
            "FleetCard");
    }

    public static async Task<T> ReadAsync<T>(HttpResponseMessage response)
    {
        var value = await response.Content.ReadFromJsonAsync<T>(JsonOptions);
        return value ?? throw new InvalidOperationException("Response body was empty.");
    }

    protected override void Dispose(bool disposing)
    {
        base.Dispose(disposing);
        if (disposing)
        {
            _connection.Dispose();
        }
    }
}
