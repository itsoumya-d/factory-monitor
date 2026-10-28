using FuelIntegrationHub.Domain.Transactions;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class FuelTransactionConfiguration : IEntityTypeConfiguration<FuelTransaction>
{
    public void Configure(EntityTypeBuilder<FuelTransaction> builder)
    {
        builder.ToTable("fuel_transactions", table =>
        {
            table.HasCheckConstraint("ck_fuel_transactions_litres", "CAST(litres AS REAL) > 0 AND CAST(litres AS REAL) <= 100000");
            table.HasCheckConstraint("ck_fuel_transactions_unit_price", "CAST(unit_price AS REAL) >= 0 AND CAST(unit_price AS REAL) <= 10000");
            table.HasCheckConstraint("ck_fuel_transactions_total", "CAST(total AS REAL) >= 0");
        });

        builder.HasKey(transaction => transaction.Id);

        builder.Property(transaction => transaction.Id).HasColumnName("id");
        builder.Property(transaction => transaction.SiteId).HasColumnName("site_id").IsRequired();
        builder.Property(transaction => transaction.DispenserId).HasColumnName("dispenser_id").IsRequired();
        builder.Property(transaction => transaction.ProductId).HasColumnName("product_id").IsRequired();
        builder.Property(transaction => transaction.ExternalId).HasColumnName("external_id").HasMaxLength(120).IsRequired();
        builder.Property(transaction => transaction.OccurredAt).HasColumnName("occurred_at").IsRequired();
        builder.Property(transaction => transaction.Litres).HasColumnName("litres").HasPrecision(18, 3).IsRequired();
        builder.Property(transaction => transaction.UnitPrice).HasColumnName("unit_price").HasPrecision(18, 4).IsRequired();
        builder.Property(transaction => transaction.Total).HasColumnName("total").HasPrecision(18, 2).IsRequired();
        builder.Property(transaction => transaction.PaymentMethod)
            .HasColumnName("payment_method")
            .HasConversion<string>()
            .HasMaxLength(32)
            .IsRequired();
        builder.Property(transaction => transaction.Status)
            .HasColumnName("status")
            .HasConversion<string>()
            .HasMaxLength(32)
            .IsRequired();
        builder.Property(transaction => transaction.CreatedAt).HasColumnName("created_at").IsRequired();
        builder.Property(transaction => transaction.Version).HasColumnName("version").IsConcurrencyToken().IsRequired();

        builder.HasIndex(transaction => new { transaction.SiteId, transaction.ExternalId })
            .IsUnique()
            .HasDatabaseName("ux_fuel_transactions_site_external");

        builder.HasIndex(transaction => new { transaction.SiteId, transaction.OccurredAt })
            .HasDatabaseName("ix_fuel_transactions_site_occurred");

        builder.HasOne(transaction => transaction.Site)
            .WithMany()
            .HasForeignKey(transaction => transaction.SiteId)
            .OnDelete(DeleteBehavior.Restrict);

        builder.HasOne(transaction => transaction.Dispenser)
            .WithMany()
            .HasForeignKey(transaction => transaction.DispenserId)
            .OnDelete(DeleteBehavior.Restrict);

        builder.HasOne(transaction => transaction.Product)
            .WithMany()
            .HasForeignKey(transaction => transaction.ProductId)
            .OnDelete(DeleteBehavior.Restrict);
    }
}
