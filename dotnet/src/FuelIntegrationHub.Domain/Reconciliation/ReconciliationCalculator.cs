using FuelIntegrationHub.Domain.Common;

namespace FuelIntegrationHub.Domain.Reconciliation;

public static class ReconciliationCalculator
{
    public static ReconciliationResult Calculate(decimal siteLitres, decimal erpLitres, decimal thresholdPercent)
    {
        if (siteLitres < 0)
        {
            throw new DomainException("Site litres cannot be negative.");
        }

        if (erpLitres < 0)
        {
            throw new DomainException("ERP litres cannot be negative.");
        }

        if (thresholdPercent is < 0 or > 100)
        {
            throw new DomainException("Threshold percent must be between 0 and 100.");
        }

        var variance = siteLitres - erpLitres;
        var variancePercent = erpLitres == 0
            ? siteLitres == 0 ? 0m : 100m
            : decimal.Round(decimal.Abs(variance) / erpLitres * 100m, 4, MidpointRounding.AwayFromZero);

        var status = variancePercent <= thresholdPercent
            ? ReconciliationStatus.Balanced
            : ReconciliationStatus.VarianceDetected;

        return new ReconciliationResult(variance, variancePercent, status);
    }
}
