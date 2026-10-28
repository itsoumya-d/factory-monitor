using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;

namespace FuelIntegrationHub.Domain.Dispensers;

public sealed class Dispenser
{
    private Dispenser()
    {
    }

    public Guid Id { get; private set; }

    public Guid SiteId { get; private set; }

    public Site Site { get; private set; } = null!;

    public Guid ProductId { get; private set; }

    public Product Product { get; private set; } = null!;

    public string Code { get; private set; } = string.Empty;

    public bool IsActive { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public static Dispenser Create(Site site, Product product, string code, DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(site);
        ArgumentNullException.ThrowIfNull(product);

        if (!site.IsActive)
        {
            throw new DomainException("Cannot add a dispenser to an inactive site.");
        }

        if (!product.IsActive)
        {
            throw new DomainException("Cannot add a dispenser for an inactive product.");
        }

        return new Dispenser
        {
            Id = Guid.NewGuid(),
            SiteId = site.Id,
            ProductId = product.Id,
            Code = Guard.NotBlank(code, nameof(code), 32).ToUpperInvariant(),
            IsActive = true,
            CreatedAt = now.ToUniversalTime()
        };
    }

    public void Deactivate()
    {
        IsActive = false;
    }
}
