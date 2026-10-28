using FuelIntegrationHub.Domain.Sites;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class SiteConfiguration : IEntityTypeConfiguration<Site>
{
    public void Configure(EntityTypeBuilder<Site> builder)
    {
        builder.ToTable("sites");
        builder.HasKey(site => site.Id);

        builder.Property(site => site.Id).HasColumnName("id");
        builder.Property(site => site.Code).HasColumnName("code").HasMaxLength(32).IsRequired();
        builder.Property(site => site.Name).HasColumnName("name").HasMaxLength(200).IsRequired();
        builder.Property(site => site.Region).HasColumnName("region").HasMaxLength(80).IsRequired();
        builder.Property(site => site.Currency).HasColumnName("currency").HasMaxLength(3).IsRequired();
        builder.Property(site => site.IsActive).HasColumnName("is_active").IsRequired();
        builder.Property(site => site.CreatedAt).HasColumnName("created_at").IsRequired();
        builder.Property(site => site.Version).HasColumnName("version").IsConcurrencyToken().IsRequired();

        builder.HasIndex(site => site.Code).IsUnique().HasDatabaseName("ux_sites_code");

        builder.HasMany(site => site.Dispensers)
            .WithOne(dispenser => dispenser.Site)
            .HasForeignKey(dispenser => dispenser.SiteId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}
