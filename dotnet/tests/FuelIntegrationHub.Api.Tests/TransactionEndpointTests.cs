using System.Globalization;
using System.Net;
using System.Text;
using FuelIntegrationHub.Api.Contracts;

namespace FuelIntegrationHub.Api.Tests;

public sealed class TransactionEndpointTests(ApiFactory factory) : IClassFixture<ApiFactory>
{
    [Fact]
    public async Task IngestReturnsCreatedAndComputesTotal()
    {
        using var client = factory.CreateAuthenticatedClient();
        var request = ApiFactory.CreateRequest(litres: 40m, unitPrice: 2.45m);

        var response = await ApiFactory.PostTransactionAsync(client, "AKL-01", request);

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var body = await ApiFactory.ReadAsync<IngestTransactionResponse>(response);
        Assert.Equal("recorded", body.Status);
        Assert.False(body.Duplicate);
        Assert.Equal(98.00m, body.Total);
        Assert.NotEqual(Guid.Empty, body.TransactionId);
    }

    [Fact]
    public async Task IngestIsIdempotentForTheSameExternalId()
    {
        using var client = factory.CreateAuthenticatedClient();
        var request = ApiFactory.CreateRequest();

        var first = await ApiFactory.PostTransactionAsync(client, "AKL-01", request);
        var second = await ApiFactory.PostTransactionAsync(client, "AKL-01", request);

        Assert.Equal(HttpStatusCode.Created, first.StatusCode);
        Assert.Equal(HttpStatusCode.OK, second.StatusCode);

        var firstBody = await ApiFactory.ReadAsync<IngestTransactionResponse>(first);
        var secondBody = await ApiFactory.ReadAsync<IngestTransactionResponse>(second);
        Assert.Equal(firstBody.TransactionId, secondBody.TransactionId);
        Assert.True(secondBody.Duplicate);
    }

    [Fact]
    public async Task IngestRejectsRequestsWithoutApiKey()
    {
        using var client = factory.CreateClient();
        var request = ApiFactory.CreateRequest();

        var response = await ApiFactory.PostTransactionAsync(client, "AKL-01", request);

        Assert.Equal(HttpStatusCode.Unauthorized, response.StatusCode);
    }

    [Fact]
    public async Task IngestRejectsInvalidWebhookSignature()
    {
        using var client = factory.CreateAuthenticatedClient();
        var body = """
                   {"externalId":"bad-signature","occurredAt":"2026-10-28T11:00:00Z","dispenserCode":"D1","productCode":"91","litres":10,"unitPrice":2.45,"paymentMethod":"Card"}
                   """;
        var request = new HttpRequestMessage(HttpMethod.Post, "/api/v1/sites/AKL-01/transactions")
        {
            Content = new StringContent(body, Encoding.UTF8, "application/json")
        };
        request.Headers.Add("X-Octane-Timestamp", DateTimeOffset.UtcNow.ToUnixTimeSeconds().ToString(CultureInfo.InvariantCulture));
        request.Headers.Add("X-Octane-Signature", "sha256=deadbeef");

        var response = await client.SendAsync(request);

        Assert.Equal(HttpStatusCode.Unauthorized, response.StatusCode);
    }

    [Fact]
    public async Task IngestReturnsNotFoundForUnknownSite()
    {
        using var client = factory.CreateAuthenticatedClient();
        var request = ApiFactory.CreateRequest();

        var response = await ApiFactory.PostTransactionAsync(client, "AKL-99", request);

        Assert.Equal(HttpStatusCode.NotFound, response.StatusCode);
    }

    [Fact]
    public async Task IngestReturnsBadRequestForInvalidDomainData()
    {
        using var client = factory.CreateAuthenticatedClient();
        var request = ApiFactory.CreateRequest(litres: 0m);

        var response = await ApiFactory.PostTransactionAsync(client, "AKL-01", request);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
    }

    [Fact]
    public async Task BatchReturnsPerItemResults()
    {
        using var client = factory.CreateAuthenticatedClient();
        var valid = ApiFactory.CreateRequest();
        var invalid = ApiFactory.CreateRequest(dispenserCode: "D9");

        var response = await ApiFactory.PostBatchAsync(
            client,
            "AKL-01",
            new BatchIngestRequest([valid, invalid]));

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await ApiFactory.ReadAsync<BatchIngestResponse>(response);
        Assert.Equal(2, body.Results.Count);
        Assert.Equal("recorded", body.Results[0].Status);
        Assert.Equal("rejected", body.Results[1].Status);
        Assert.NotNull(body.Results[1].Error);
    }

    [Fact]
    public async Task QueryReturnsTransactionsInsideTheRequestedWindow()
    {
        using var client = factory.CreateAuthenticatedClient();
        var externalId = Guid.NewGuid().ToString("N");
        var occurredAt = DateTimeOffset.UtcNow.AddMinutes(-10);
        var request = ApiFactory.CreateRequest(externalId: externalId, occurredAt: occurredAt);

        var ingest = await ApiFactory.PostTransactionAsync(client, "HLZ-01", request);
        Assert.Equal(HttpStatusCode.Created, ingest.StatusCode);

        var from = Uri.EscapeDataString(DateTimeOffset.UtcNow.AddHours(-1).ToString("O", CultureInfo.InvariantCulture));
        var to = Uri.EscapeDataString(DateTimeOffset.UtcNow.AddHours(1).ToString("O", CultureInfo.InvariantCulture));
        var response = await client.GetAsync(
            $"/api/v1/sites/HLZ-01/transactions?from={from}&to={to}&page=1&pageSize=50");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await ApiFactory.ReadAsync<TransactionQueryResponse>(response);
        Assert.Contains(body.Items, item => item.ExternalId == externalId);
        Assert.Equal(50, body.PageSize);
    }
}
