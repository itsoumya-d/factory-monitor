using FuelIntegrationHub.Api.Contracts;
using FuelIntegrationHub.Infrastructure.Reconciliation;

namespace FuelIntegrationHub.Api.Endpoints;

public static class ReconciliationEndpoints
{
    public static IEndpointRouteBuilder MapReconciliationEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app
            .MapGroup("/api/v1/sites/{siteCode}/reconciliations")
            .RequireAuthorization()
            .WithTags("Reconciliation");

        group.MapPost("/", RunAsync)
            .WithName("RunReconciliation");

        group.MapGet("/", ListAsync)
            .WithName("ListReconciliations");

        return app;
    }

    private static async Task<IResult> RunAsync(
        string siteCode,
        ReconciliationRequest request,
        FuelReconciliationService reconciliation,
        CancellationToken cancellationToken)
    {
        var run = await reconciliation.RunAsync(
            siteCode,
            request.PeriodStart,
            request.PeriodEnd,
            request.ErpLitres,
            request.ThresholdPercent,
            cancellationToken);

        return Results.Created(
            $"/api/v1/sites/{siteCode}/reconciliations/{run.Id}",
            ToResponse(siteCode, run));
    }

    private static async Task<IResult> ListAsync(
        string siteCode,
        int? limit,
        FuelReconciliationService reconciliation,
        CancellationToken cancellationToken)
    {
        var runs = await reconciliation.ListAsync(siteCode, limit ?? 20, cancellationToken);
        return Results.Ok(runs.Select(run => ToResponse(siteCode, run)).ToArray());
    }

    private static ReconciliationResponse ToResponse(
        string siteCode,
        Domain.Reconciliation.ReconciliationRun run)
    {
        return new ReconciliationResponse(
            run.Id,
            siteCode.Trim().ToUpperInvariant(),
            run.PeriodStart,
            run.PeriodEnd,
            run.SiteLitres,
            run.ErpLitres,
            run.VarianceLitres,
            run.VariancePercent,
            run.ThresholdPercent,
            run.Status.ToString(),
            run.CreatedAt);
    }
}
