using FuelIntegrationHub.Domain.Reconciliation;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class ReconciliationRunConfiguration : IEntityTypeConfiguration<ReconciliationRun>
{
    public void Configure(EntityTypeBuilder<ReconciliationRun> builder)
    {
        builder.ToTable("reconciliation_runs", table =>
        {
            table.HasCheckConstraint("ck_reconciliation_runs_period", "period_start < period_end");
            table.HasCheckConstraint("ck_reconciliation_runs_litres", "CAST(site_litres AS REAL) >= 0 AND CAST(erp_litres AS REAL) >= 0");
            table.HasCheckConstraint("ck_reconciliation_runs_threshold", "CAST(threshold_percent AS REAL) >= 0 AND CAST(threshold_percent AS REAL) <= 100");
        });

        builder.HasKey(run => run.Id);

        builder.Property(run => run.Id).HasColumnName("id");
        builder.Property(run => run.SiteId).HasColumnName("site_id").IsRequired();
        builder.Property(run => run.PeriodStart).HasColumnName("period_start").IsRequired();
        builder.Property(run => run.PeriodEnd).HasColumnName("period_end").IsRequired();
        builder.Property(run => run.SiteLitres).HasColumnName("site_litres").HasPrecision(18, 3).IsRequired();
        builder.Property(run => run.ErpLitres).HasColumnName("erp_litres").HasPrecision(18, 3).IsRequired();
        builder.Property(run => run.VarianceLitres).HasColumnName("variance_litres").HasPrecision(18, 3).IsRequired();
        builder.Property(run => run.VariancePercent).HasColumnName("variance_percent").HasPrecision(9, 4).IsRequired();
        builder.Property(run => run.ThresholdPercent).HasColumnName("threshold_percent").HasPrecision(9, 4).IsRequired();
        builder.Property(run => run.Status)
            .HasColumnName("status")
            .HasConversion<string>()
            .HasMaxLength(32)
            .IsRequired();
        builder.Property(run => run.CreatedAt).HasColumnName("created_at").IsRequired();

        builder.HasIndex(run => new { run.SiteId, run.PeriodEnd })
            .HasDatabaseName("ix_reconciliation_runs_site_period");

        builder.HasOne(run => run.Site)
            .WithMany()
            .HasForeignKey(run => run.SiteId)
            .OnDelete(DeleteBehavior.Restrict);
    }
}
