using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Diagnostics.HealthChecks;

namespace FuelIntegrationHub.Infrastructure.Persistence;

public sealed class DatabaseHealthCheck(IServiceScopeFactory scopeFactory) : IHealthCheck
{
    public async Task<HealthCheckResult> CheckHealthAsync(
        HealthCheckContext context,
        CancellationToken cancellationToken = default)
    {
        await using var scope = scopeFactory.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<FuelDbContext>();

        return await db.Database.CanConnectAsync(cancellationToken)
            ? HealthCheckResult.Healthy("SQLite database is reachable.")
            : HealthCheckResult.Unhealthy("SQLite database is not reachable.");
    }
}
