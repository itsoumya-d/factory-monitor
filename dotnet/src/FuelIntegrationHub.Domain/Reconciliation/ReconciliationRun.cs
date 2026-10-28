using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Sites;

namespace FuelIntegrationHub.Domain.Reconciliation;

public sealed class ReconciliationRun
{
    private ReconciliationRun()
    {
    }

    public Guid Id { get; private set; }

    public Guid SiteId { get; private set; }

    public Site Site { get; private set; } = null!;

    public DateTimeOffset PeriodStart { get; private set; }

    public DateTimeOffset PeriodEnd { get; private set; }

    public decimal SiteLitres { get; private set; }

    public decimal ErpLitres { get; private set; }

    public decimal VarianceLitres { get; private set; }

    public decimal VariancePercent { get; private set; }

    public decimal ThresholdPercent { get; private set; }

    public ReconciliationStatus Status { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public static ReconciliationRun Create(
        Site site,
        DateTimeOffset periodStart,
        DateTimeOffset periodEnd,
        decimal siteLitres,
        decimal erpLitres,
        decimal thresholdPercent,
        DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(site);

        if (periodStart >= periodEnd)
        {
            throw new DomainException("PeriodStart must be before PeriodEnd.");
        }

        var result = ReconciliationCalculator.Calculate(siteLitres, erpLitres, thresholdPercent);

        return new ReconciliationRun
        {
            Id = Guid.NewGuid(),
            SiteId = site.Id,
            PeriodStart = periodStart.ToUniversalTime(),
            PeriodEnd = periodEnd.ToUniversalTime(),
            SiteLitres = siteLitres,
            ErpLitres = erpLitres,
            VarianceLitres = result.VarianceLitres,
            VariancePercent = result.VariancePercent,
            ThresholdPercent = thresholdPercent,
            Status = result.Status,
            CreatedAt = now.ToUniversalTime()
        };
    }
}
