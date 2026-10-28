using FuelIntegrationHub.Api.Contracts;
using FuelIntegrationHub.Domain.Outbox;
using FuelIntegrationHub.Domain.Reconciliation;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Api.Endpoints;

public static class OpsEndpoints
{
    public static IEndpointRouteBuilder MapOpsEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app
            .MapGroup("/api/v1/ops")
            .RequireAuthorization()
            .WithTags("Operations");

        group.MapGet("/outbox", ListOutboxAsync)
            .WithName("ListOutbox");

        group.MapPost("/outbox/{id:guid}/retry", RetryOutboxAsync)
            .WithName("RetryOutboxMessage");

        group.MapGet("/metrics", GetMetricsAsync)
            .WithName("GetOpsMetrics");

        var sites = app
            .MapGroup("/api/v1/sites")
            .RequireAuthorization()
            .WithTags("Sites");

        sites.MapGet("/", ListSitesAsync)
            .WithName("ListSites");

        return app;
    }

    private static async Task<IResult> ListOutboxAsync(
        string? status,
        int? limit,
        FuelDbContext db,
        CancellationToken cancellationToken)
    {
        OutboxStatus? filter = null;
        if (!string.IsNullOrWhiteSpace(status))
        {
            if (!Enum.TryParse<OutboxStatus>(status, ignoreCase: true, out var parsed))
            {
                return Results.Problem(
                    statusCode: StatusCodes.Status400BadRequest,
                    title: "Invalid outbox status",
                    detail: "Use Pending, Processing, Sent, or DeadLetter.");
            }

            filter = parsed;
        }

        var query = db.OutboxMessages.AsNoTracking();
        if (filter is not null)
        {
            query = query.Where(message => message.Status == filter);
        }

        var rows = await query
            .OrderByDescending(message => message.OccurredAt)
            .Take(Math.Clamp(limit ?? 50, 1, 200))
            .Select(message => new
            {
                message.Id,
                message.Type,
                message.IdempotencyKey,
                message.Status,
                message.Attempts,
                message.OccurredAt,
                message.NextAttemptAt,
                message.ProcessedAt,
                message.LastError
            })
            .ToListAsync(cancellationToken);

        var items = rows
            .Select(row => new OutboxItemResponse(
                row.Id,
                row.Type,
                row.IdempotencyKey,
                row.Status.ToString(),
                row.Attempts,
                row.OccurredAt,
                row.NextAttemptAt,
                row.ProcessedAt,
                row.LastError))
            .ToArray();

        return Results.Ok(new OutboxListResponse(filter?.ToString(), items.Length, items));
    }

    private static async Task<IResult> RetryOutboxAsync(
        Guid id,
        FuelDbContext db,
        TimeProvider timeProvider,
        CancellationToken cancellationToken)
    {
        var message = await db.OutboxMessages
            .SingleOrDefaultAsync(candidate => candidate.Id == id, cancellationToken);

        if (message is null)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status404NotFound,
                title: "Outbox message not found",
                detail: $"Outbox message '{id}' was not found.");
        }

        message.Requeue(timeProvider.GetUtcNow());
        await db.SaveChangesAsync(cancellationToken);

        return Results.Ok(new OutboxRetryResponse(message.Id, message.Status.ToString(), message.NextAttemptAt));
    }

    private static async Task<IResult> GetMetricsAsync(
        FuelDbContext db,
        TimeProvider timeProvider,
        CancellationToken cancellationToken)
    {
        var now = timeProvider.GetUtcNow();
        var todayUtc = new DateTimeOffset(now.UtcDateTime.Date, TimeSpan.Zero);

        var metrics = new OpsMetricsResponse(
            Sites: await db.Sites.CountAsync(cancellationToken),
            Dispensers: await db.Dispensers.CountAsync(cancellationToken),
            Transactions: await db.FuelTransactions.CountAsync(cancellationToken),
            TransactionsTodayUtc: await db.FuelTransactions
                .CountAsync(transaction => transaction.OccurredAt >= todayUtc, cancellationToken),
            OutboxPending: await db.OutboxMessages
                .CountAsync(message => message.Status == OutboxStatus.Pending, cancellationToken),
            OutboxSent: await db.OutboxMessages
                .CountAsync(message => message.Status == OutboxStatus.Sent, cancellationToken),
            OutboxDeadLetter: await db.OutboxMessages
                .CountAsync(message => message.Status == OutboxStatus.DeadLetter, cancellationToken),
            ReconciliationRuns: await db.ReconciliationRuns.CountAsync(cancellationToken),
            ReconciliationVariances: await db.ReconciliationRuns
                .CountAsync(run => run.Status == ReconciliationStatus.VarianceDetected, cancellationToken),
            LastReconciliationAt: await db.ReconciliationRuns
                .OrderByDescending(run => run.CreatedAt)
                .Select(run => (DateTimeOffset?)run.CreatedAt)
                .FirstOrDefaultAsync(cancellationToken));

        return Results.Ok(metrics);
    }

    private static async Task<IResult> ListSitesAsync(
        FuelDbContext db,
        CancellationToken cancellationToken)
    {
        var sites = await db.Sites
            .AsNoTracking()
            .OrderBy(site => site.Code)
            .Select(site => new SiteResponse(
                site.Id,
                site.Code,
                site.Name,
                site.Region,
                site.Currency,
                site.IsActive,
                site.Dispensers.Count))
            .ToListAsync(cancellationToken);

        return Results.Ok(sites);
    }
}
