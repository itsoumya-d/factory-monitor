using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Outbox;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Reconciliation;
using FuelIntegrationHub.Domain.Sites;
using FuelIntegrationHub.Domain.Transactions;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Storage.ValueConversion;

namespace FuelIntegrationHub.Infrastructure.Persistence;

public sealed class FuelDbContext(DbContextOptions<FuelDbContext> options) : DbContext(options)
{
    public DbSet<Site> Sites => Set<Site>();

    public DbSet<Product> Products => Set<Product>();

    public DbSet<Dispenser> Dispensers => Set<Dispenser>();

    public DbSet<FuelTransaction> FuelTransactions => Set<FuelTransaction>();

    public DbSet<ReconciliationRun> ReconciliationRuns => Set<ReconciliationRun>();

    public DbSet<OutboxMessage> OutboxMessages => Set<OutboxMessage>();

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.ApplyConfigurationsFromAssembly(typeof(FuelDbContext).Assembly);
    }

    protected override void ConfigureConventions(ModelConfigurationBuilder configurationBuilder)
    {
        configurationBuilder.Properties<DateTimeOffset>()
            .HaveConversion<DateTimeOffsetToUnixMillisecondsConverter>();
    }

    public override int SaveChanges()
    {
        ApplyConcurrencyVersions();
        return base.SaveChanges();
    }

    public override Task<int> SaveChangesAsync(CancellationToken cancellationToken = default)
    {
        ApplyConcurrencyVersions();
        return base.SaveChangesAsync(cancellationToken);
    }

    private void ApplyConcurrencyVersions()
    {
        foreach (var entry in ChangeTracker.Entries())
        {
            if (entry.State != EntityState.Modified)
            {
                continue;
            }

            var version = entry.Properties.FirstOrDefault(property =>
                property.Metadata.Name == "Version" && property.Metadata.ClrType == typeof(int));

            if (version is not null && version.CurrentValue is int current)
            {
                version.CurrentValue = current + 1;
            }
        }
    }
}
