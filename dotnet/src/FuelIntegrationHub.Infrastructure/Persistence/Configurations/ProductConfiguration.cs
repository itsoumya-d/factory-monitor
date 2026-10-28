using FuelIntegrationHub.Domain.Products;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class ProductConfiguration : IEntityTypeConfiguration<Product>
{
    public void Configure(EntityTypeBuilder<Product> builder)
    {
        builder.ToTable("products");
        builder.HasKey(product => product.Id);

        builder.Property(product => product.Id).HasColumnName("id");
        builder.Property(product => product.Code).HasColumnName("code").HasMaxLength(32).IsRequired();
        builder.Property(product => product.Name).HasColumnName("name").HasMaxLength(120).IsRequired();
        builder.Property(product => product.Unit).HasColumnName("unit").HasMaxLength(8).IsRequired();
        builder.Property(product => product.IsActive).HasColumnName("is_active").IsRequired();
        builder.Property(product => product.CreatedAt).HasColumnName("created_at").IsRequired();

        builder.HasIndex(product => product.Code).IsUnique().HasDatabaseName("ux_products_code");
    }
}
