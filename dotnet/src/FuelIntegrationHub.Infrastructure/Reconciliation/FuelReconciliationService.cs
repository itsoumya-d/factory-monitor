using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Reconciliation;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Infrastructure.Reconciliation;

public sealed class FuelReconciliationService(FuelDbContext db, TimeProvider timeProvider)
{
    public async Task<ReconciliationRun> RunAsync(
        string siteCode,
        DateTimeOffset periodStart,
        DateTimeOffset periodEnd,
        decimal erpLitres,
        decimal thresholdPercent,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(siteCode))
        {
            throw new DomainException("Site code is required.");
        }

        if (periodStart >= periodEnd)
        {
            throw new DomainException("PeriodStart must be before PeriodEnd.");
        }

        var normalizedCode = siteCode.Trim().ToUpperInvariant();
        var site = await db.Sites.SingleOrDefaultAsync(
            candidate => candidate.Code == normalizedCode,
            cancellationToken)
            ?? throw new KeyNotFoundException($"Site '{siteCode}' was not found.");

        var start = periodStart.ToUniversalTime();
        var end = periodEnd.ToUniversalTime();

        var siteLitres = await db.FuelTransactions
            .Where(transaction =>
                transaction.SiteId == site.Id
                && transaction.Status == Domain.Transactions.TransactionStatus.Recorded
                && transaction.OccurredAt >= start
                && transaction.OccurredAt < end)
            .SumAsync(transaction => (decimal?)transaction.Litres, cancellationToken) ?? 0m;

        var run = ReconciliationRun.Create(
            site,
            start,
            end,
            siteLitres,
            erpLitres,
            thresholdPercent,
            timeProvider.GetUtcNow());

        db.ReconciliationRuns.Add(run);
        await db.SaveChangesAsync(cancellationToken);

        return run;
    }

    public async Task<IReadOnlyList<ReconciliationRun>> ListAsync(
        string siteCode,
        int limit,
        CancellationToken cancellationToken)
    {
        var normalizedCode = siteCode.Trim().ToUpperInvariant();
        var siteExists = await db.Sites.AnyAsync(site => site.Code == normalizedCode, cancellationToken);
        if (!siteExists)
        {
            throw new KeyNotFoundException($"Site '{siteCode}' was not found.");
        }

        return await db.ReconciliationRuns
            .AsNoTracking()
            .Where(run => run.Site.Code == normalizedCode)
            .OrderByDescending(run => run.PeriodEnd)
            .Take(Math.Clamp(limit, 1, 100))
            .ToListAsync(cancellationToken);
    }
}
