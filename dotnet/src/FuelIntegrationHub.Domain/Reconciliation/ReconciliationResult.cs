namespace FuelIntegrationHub.Domain.Reconciliation;

public readonly record struct ReconciliationResult(
    decimal VarianceLitres,
    decimal VariancePercent,
    ReconciliationStatus Status);
