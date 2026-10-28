using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Reconciliation;

namespace FuelIntegrationHub.Domain.Tests;

public sealed class ReconciliationCalculatorTests
{
    [Fact]
    public void CalculateReturnsBalancedWithinThreshold()
    {
        var result = ReconciliationCalculator.Calculate(1000m, 999.5m, 0.5m);

        Assert.Equal(0.5m, result.VarianceLitres);
        Assert.Equal(0.05m, result.VariancePercent);
        Assert.Equal(ReconciliationStatus.Balanced, result.Status);
    }

    [Fact]
    public void CalculateFlagsVarianceAboveThreshold()
    {
        var result = ReconciliationCalculator.Calculate(1000m, 995m, 0.5m);

        Assert.Equal(5m, result.VarianceLitres);
        Assert.Equal(0.5025m, result.VariancePercent);
        Assert.Equal(ReconciliationStatus.VarianceDetected, result.Status);
    }

    [Fact]
    public void CalculateHandlesZeroErpLitres()
    {
        var result = ReconciliationCalculator.Calculate(10m, 0m, 0.5m);

        Assert.Equal(10m, result.VarianceLitres);
        Assert.Equal(100m, result.VariancePercent);
        Assert.Equal(ReconciliationStatus.VarianceDetected, result.Status);
    }

    [Fact]
    public void CalculateHandlesZeroVsZero()
    {
        var result = ReconciliationCalculator.Calculate(0m, 0m, 0m);

        Assert.Equal(0m, result.VarianceLitres);
        Assert.Equal(0m, result.VariancePercent);
        Assert.Equal(ReconciliationStatus.Balanced, result.Status);
    }

    [Fact]
    public void CalculateRejectsNegativeInputs()
    {
        Assert.Throws<DomainException>(() => ReconciliationCalculator.Calculate(-1m, 0m, 0.5m));
        Assert.Throws<DomainException>(() => ReconciliationCalculator.Calculate(0m, -1m, 0.5m));
        Assert.Throws<DomainException>(() => ReconciliationCalculator.Calculate(0m, 0m, 101m));
    }
}
