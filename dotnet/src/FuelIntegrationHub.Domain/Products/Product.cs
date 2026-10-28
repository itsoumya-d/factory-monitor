using FuelIntegrationHub.Domain.Common;

namespace FuelIntegrationHub.Domain.Products;

public sealed class Product
{
    private Product()
    {
    }

    public Guid Id { get; private set; }

    public string Code { get; private set; } = string.Empty;

    public string Name { get; private set; } = string.Empty;

    public string Unit { get; private set; } = "L";

    public bool IsActive { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public static Product Create(string code, string name, string unit, DateTimeOffset now)
    {
        return new Product
        {
            Id = Guid.NewGuid(),
            Code = Guard.NotBlank(code, nameof(code), 32).ToUpperInvariant(),
            Name = Guard.NotBlank(name, nameof(name), 120),
            Unit = Guard.NotBlank(unit, nameof(unit), 8).ToUpperInvariant(),
            IsActive = true,
            CreatedAt = now.ToUniversalTime()
        };
    }

    public void Deactivate()
    {
        IsActive = false;
    }
}
