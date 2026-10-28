using System.Net;
using System.Net.Http.Json;
using FuelIntegrationHub.Api.Contracts;

namespace FuelIntegrationHub.Api.Tests;

public sealed class ReconciliationEndpointTests(ApiFactory factory) : IClassFixture<ApiFactory>
{
    [Fact]
    public async Task ReconciliationFlagsVarianceAboveThreshold()
    {
        using var client = factory.CreateAuthenticatedClient();
        var start = DateTimeOffset.UtcNow.AddHours(-2);
        await IngestAsync(client, 25m, start.AddSeconds(5));
        await IngestAsync(client, 15m, start.AddSeconds(10));

        var response = await client.PostAsJsonAsync(
            "/api/v1/sites/AKL-01/reconciliations",
            new ReconciliationRequest(
                start,
                start.AddMinutes(1),
                ErpLitres: 30m,
                ThresholdPercent: 0.5m),
            ApiFactory.JsonOptions);

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var body = await ApiFactory.ReadAsync<ReconciliationResponse>(response);
        Assert.Equal(40m, body.SiteLitres);
        Assert.Equal(10m, body.VarianceLitres);
        Assert.Equal("VarianceDetected", body.Status);
    }

    [Fact]
    public async Task ReconciliationBalancesWithinThreshold()
    {
        using var client = factory.CreateAuthenticatedClient();
        var start = DateTimeOffset.UtcNow.AddHours(-3);
        await IngestAsync(client, 20m, start.AddSeconds(5));
        await IngestAsync(client, 20m, start.AddSeconds(10));

        var response = await client.PostAsJsonAsync(
            "/api/v1/sites/AKL-01/reconciliations",
            new ReconciliationRequest(
                start,
                start.AddMinutes(1),
                ErpLitres: 40m,
                ThresholdPercent: 0.5m),
            ApiFactory.JsonOptions);

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var body = await ApiFactory.ReadAsync<ReconciliationResponse>(response);
        Assert.Equal(0m, body.VarianceLitres);
        Assert.Equal("Balanced", body.Status);
    }

    [Fact]
    public async Task ReconciliationListReturnsLatestRuns()
    {
        using var client = factory.CreateAuthenticatedClient();
        var start = DateTimeOffset.UtcNow.AddHours(-4);
        await IngestAsync(client, 5m, start.AddSeconds(5));

        await client.PostAsJsonAsync(
            "/api/v1/sites/AKL-01/reconciliations",
            new ReconciliationRequest(
                start,
                start.AddMinutes(1),
                ErpLitres: 5m),
            ApiFactory.JsonOptions);

        var response = await client.GetAsync("/api/v1/sites/AKL-01/reconciliations?limit=10");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await ApiFactory.ReadAsync<ReconciliationResponse[]>(response);
        Assert.NotEmpty(body);
    }

    private static async Task IngestAsync(HttpClient client, decimal litres, DateTimeOffset occurredAt)
    {
        var response = await ApiFactory.PostTransactionAsync(
            client,
            "AKL-01",
            ApiFactory.CreateRequest(litres: litres, occurredAt: occurredAt));

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
    }
}
