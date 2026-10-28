using System.Net;
using FuelIntegrationHub.Api.Contracts;
using FuelIntegrationHub.Domain.Outbox;
using FuelIntegrationHub.Infrastructure.Integrations;
using FuelIntegrationHub.Infrastructure.Outbox;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;

namespace FuelIntegrationHub.Api.Tests;

public sealed class OpsEndpointTests(ApiFactory factory) : IClassFixture<ApiFactory>
{
    [Fact]
    public async Task OutboxProcessorDeliversPendingMessagesToErp()
    {
        using var client = factory.CreateAuthenticatedClient();
        var externalId = Guid.NewGuid().ToString("N");

        var ingest = await ApiFactory.PostTransactionAsync(
            client,
            "AKL-01",
            ApiFactory.CreateRequest(externalId: externalId));
        Assert.Equal(HttpStatusCode.Created, ingest.StatusCode);

        var pendingResponse = await client.GetAsync("/api/v1/ops/outbox?status=Pending&limit=200");
        var pending = await ApiFactory.ReadAsync<OutboxListResponse>(pendingResponse);
        Assert.Contains(pending.Items, item => item.IdempotencyKey.Contains(externalId, StringComparison.Ordinal));

        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var processor = scope.ServiceProvider.GetRequiredService<OutboxProcessor>();
            await processor.ProcessBatchAsync(CancellationToken.None);
        }

        var sentResponse = await client.GetAsync("/api/v1/ops/outbox?status=Sent&limit=200");
        var sent = await ApiFactory.ReadAsync<OutboxListResponse>(sentResponse);
        Assert.Contains(sent.Items, item => item.IdempotencyKey.Contains(externalId, StringComparison.Ordinal));

        await using var verificationScope = factory.Services.CreateAsyncScope();
        var sink = verificationScope.ServiceProvider.GetRequiredService<InMemoryErpSink>();
        Assert.Contains(sink.Events, transactionEvent => transactionEvent.ExternalId == externalId);
    }

    [Fact]
    public async Task MetricsReturnsOperationalCounts()
    {
        using var client = factory.CreateAuthenticatedClient();
        var ingest = await ApiFactory.PostTransactionAsync(
            client,
            "AKL-01",
            ApiFactory.CreateRequest());
        Assert.Equal(HttpStatusCode.Created, ingest.StatusCode);

        var response = await client.GetAsync("/api/v1/ops/metrics");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var metrics = await ApiFactory.ReadAsync<OpsMetricsResponse>(response);
        Assert.True(metrics.Sites >= 2);
        Assert.True(metrics.Transactions >= 1);
        Assert.True(metrics.OutboxPending >= 1);
    }

    [Fact]
    public async Task RetryRequeuesDeadLetterMessage()
    {
        using var client = factory.CreateAuthenticatedClient();
        var externalId = Guid.NewGuid().ToString("N");
        var ingest = await ApiFactory.PostTransactionAsync(
            client,
            "AKL-01",
            ApiFactory.CreateRequest(externalId: externalId));
        Assert.Equal(HttpStatusCode.Created, ingest.StatusCode);

        Guid messageId;
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<FuelDbContext>();
            var message = await db.OutboxMessages
                .SingleAsync(candidate => candidate.IdempotencyKey.Contains(externalId));
            message.MarkFailed("forced failure", DateTimeOffset.UtcNow, maxAttempts: 1);
            await db.SaveChangesAsync();
            messageId = message.Id;
        }

        var response = await client.PostAsync($"/api/v1/ops/outbox/{messageId}/retry", content: null);

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await ApiFactory.ReadAsync<OutboxRetryResponse>(response);
        Assert.Equal("Pending", body.Status);
    }

    [Fact]
    public async Task HealthIsPubliclyReachable()
    {
        using var client = factory.CreateClient();

        var response = await client.GetAsync("/health");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
    }
}
