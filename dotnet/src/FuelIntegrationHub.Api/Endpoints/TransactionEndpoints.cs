using FuelIntegrationHub.Api.Contracts;
using FuelIntegrationHub.Domain.Transactions;
using FuelIntegrationHub.Infrastructure.Ingestion;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Api.Endpoints;

public static class TransactionEndpoints
{
    public static IEndpointRouteBuilder MapTransactionEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app
            .MapGroup("/api/v1/sites/{siteCode}/transactions")
            .RequireAuthorization()
            .WithTags("Transactions");

        group.MapPost("/", IngestAsync)
            .WithName("IngestTransaction");

        group.MapPost("/batch", IngestBatchAsync)
            .WithName("IngestTransactionBatch");

        group.MapGet("/", QueryAsync)
            .WithName("QueryTransactions");

        return app;
    }

    private static async Task<IResult> IngestAsync(
        string siteCode,
        IngestTransactionRequest request,
        FuelIngestionService ingestion,
        CancellationToken cancellationToken)
    {
        var command = ToCommand(request);
        var result = await ingestion.IngestAsync(siteCode, command, cancellationToken);
        var response = new IngestTransactionResponse(
            result.TransactionId,
            result.ExternalId,
            result.Status,
            result.Total,
            result.Duplicate);

        return result.Duplicate
            ? Results.Ok(response)
            : Results.Created(
                $"/api/v1/sites/{siteCode}/transactions/{result.TransactionId}",
                response);
    }

    private static async Task<IResult> IngestBatchAsync(
        string siteCode,
        BatchIngestRequest request,
        FuelIngestionService ingestion,
        CancellationToken cancellationToken)
    {
        if (request.Transactions.Count == 0)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status400BadRequest,
                title: "Empty batch",
                detail: "At least one transaction is required.");
        }

        if (request.Transactions.Count > 500)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status400BadRequest,
                title: "Batch too large",
                detail: "A maximum of 500 transactions can be submitted per batch.");
        }

        var commands = request.Transactions.Select(ToCommand).ToArray();
        var results = await ingestion.IngestBatchAsync(siteCode, commands, cancellationToken);

        return Results.Ok(new BatchIngestResponse(
            results
                .Select(result => new BatchIngestItemResponse(
                    result.Index,
                    result.ExternalId,
                    result.Status,
                    result.TransactionId,
                    result.Total,
                    result.Error))
                .ToArray()));
    }

    private static async Task<IResult> QueryAsync(
        string siteCode,
        DateTimeOffset? from,
        DateTimeOffset? to,
        int? page,
        int? pageSize,
        FuelDbContext db,
        TimeProvider timeProvider,
        CancellationToken cancellationToken)
    {
        var normalizedCode = siteCode.Trim().ToUpperInvariant();
        var siteExists = await db.Sites
            .AsNoTracking()
            .AnyAsync(site => site.Code == normalizedCode, cancellationToken);

        if (!siteExists)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status404NotFound,
                title: "Site not found",
                detail: $"Site '{siteCode}' was not found.");
        }

        var now = timeProvider.GetUtcNow();
        var rangeStart = (from ?? now.AddDays(-7)).ToUniversalTime();
        var rangeEnd = (to ?? now).ToUniversalTime();

        if (rangeStart >= rangeEnd)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status400BadRequest,
                title: "Invalid time range",
                detail: "The 'from' timestamp must be before the 'to' timestamp.");
        }

        var currentPage = Math.Max(1, page ?? 1);
        var currentPageSize = Math.Clamp(pageSize ?? 50, 1, 200);

        var query = db.FuelTransactions
            .AsNoTracking()
            .Where(transaction =>
                transaction.Site.Code == normalizedCode
                && transaction.OccurredAt >= rangeStart
                && transaction.OccurredAt < rangeEnd);

        var totalCount = await query.CountAsync(cancellationToken);
        var rows = await query
            .OrderByDescending(transaction => transaction.OccurredAt)
            .Skip((currentPage - 1) * currentPageSize)
            .Take(currentPageSize)
            .Select(transaction => new
            {
                transaction.Id,
                transaction.ExternalId,
                transaction.OccurredAt,
                DispenserCode = transaction.Dispenser.Code,
                ProductCode = transaction.Product.Code,
                transaction.Litres,
                transaction.UnitPrice,
                transaction.Total,
                transaction.PaymentMethod,
                transaction.Status
            })
            .ToListAsync(cancellationToken);

        var items = rows
            .Select(row => new TransactionQueryItem(
                row.Id,
                row.ExternalId,
                row.OccurredAt,
                row.DispenserCode,
                row.ProductCode,
                row.Litres,
                row.UnitPrice,
                row.Total,
                row.PaymentMethod.ToString(),
                row.Status.ToString()))
            .ToArray();

        return Results.Ok(new TransactionQueryResponse(
            normalizedCode,
            rangeStart,
            rangeEnd,
            currentPage,
            currentPageSize,
            totalCount,
            items));
    }

    private static IngestTransactionCommand ToCommand(IngestTransactionRequest request)
    {
        var paymentMethod = Enum.TryParse<PaymentMethod>(request.PaymentMethod, ignoreCase: true, out var parsed)
            ? parsed
            : PaymentMethod.Unknown;

        return new IngestTransactionCommand(
            request.ExternalId,
            request.OccurredAt,
            request.DispenserCode,
            request.ProductCode,
            request.Litres,
            request.UnitPrice,
            paymentMethod);
    }
}
