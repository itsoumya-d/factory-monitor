using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;

namespace FuelIntegrationHub.Infrastructure.Persistence;

public static class DatabaseInitializer
{
    public static async Task InitializeAsync(
        IServiceProvider services,
        IConfiguration configuration,
        CancellationToken cancellationToken = default)
    {
        await using var scope = services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<FuelDbContext>();

        if (configuration.GetValue("Database:EnsureCreated", true))
        {
            await db.Database.EnsureCreatedAsync(cancellationToken);
        }

        if (configuration.GetValue("Database:Seed", true))
        {
            await DatabaseSeeder.SeedAsync(db, TimeProvider.System.GetUtcNow(), cancellationToken);
        }
    }
}
