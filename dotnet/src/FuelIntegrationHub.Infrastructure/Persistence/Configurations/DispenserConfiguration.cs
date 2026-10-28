using FuelIntegrationHub.Domain.Dispensers;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class DispenserConfiguration : IEntityTypeConfiguration<Dispenser>
{
    public void Configure(EntityTypeBuilder<Dispenser> builder)
    {
        builder.ToTable("dispensers");
        builder.HasKey(dispenser => dispenser.Id);

        builder.Property(dispenser => dispenser.Id).HasColumnName("id");
        builder.Property(dispenser => dispenser.SiteId).HasColumnName("site_id").IsRequired();
        builder.Property(dispenser => dispenser.ProductId).HasColumnName("product_id").IsRequired();
        builder.Property(dispenser => dispenser.Code).HasColumnName("code").HasMaxLength(32).IsRequired();
        builder.Property(dispenser => dispenser.IsActive).HasColumnName("is_active").IsRequired();
        builder.Property(dispenser => dispenser.CreatedAt).HasColumnName("created_at").IsRequired();

        builder.HasIndex(dispenser => new { dispenser.SiteId, dispenser.Code })
            .IsUnique()
            .HasDatabaseName("ux_dispensers_site_code");

        builder.HasOne(dispenser => dispenser.Site)
            .WithMany(site => site.Dispensers)
            .HasForeignKey(dispenser => dispenser.SiteId)
            .OnDelete(DeleteBehavior.Cascade);

        builder.HasOne(dispenser => dispenser.Product)
            .WithMany()
            .HasForeignKey(dispenser => dispenser.ProductId)
            .OnDelete(DeleteBehavior.Restrict);
    }
}
