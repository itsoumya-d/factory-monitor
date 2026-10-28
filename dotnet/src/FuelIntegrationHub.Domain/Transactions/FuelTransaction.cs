using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;

namespace FuelIntegrationHub.Domain.Transactions;

public sealed class FuelTransaction
{
    private FuelTransaction()
    {
    }

    public Guid Id { get; private set; }

    public Guid SiteId { get; private set; }

    public Site Site { get; private set; } = null!;

    public Guid DispenserId { get; private set; }

    public Dispenser Dispenser { get; private set; } = null!;

    public Guid ProductId { get; private set; }

    public Product Product { get; private set; } = null!;

    public string ExternalId { get; private set; } = string.Empty;

    public DateTimeOffset OccurredAt { get; private set; }

    public decimal Litres { get; private set; }

    public decimal UnitPrice { get; private set; }

    public decimal Total { get; private set; }

    public PaymentMethod PaymentMethod { get; private set; }

    public TransactionStatus Status { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public int Version { get; private set; }

    public static FuelTransaction Record(
        Site site,
        Dispenser dispenser,
        Product product,
        string externalId,
        DateTimeOffset occurredAt,
        decimal litres,
        decimal unitPrice,
        PaymentMethod paymentMethod,
        DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(site);
        ArgumentNullException.ThrowIfNull(dispenser);
        ArgumentNullException.ThrowIfNull(product);

        if (!site.IsActive)
        {
            throw new DomainException("Cannot record a transaction for an inactive site.");
        }

        if (!dispenser.IsActive || dispenser.SiteId != site.Id)
        {
            throw new DomainException("The dispenser is not active for this site.");
        }

        if (!product.IsActive || dispenser.ProductId != product.Id)
        {
            throw new DomainException("The dispenser is not configured for this product.");
        }

        var validatedLitres = Guard.Positive(litres, nameof(litres), 100_000m);
        var validatedUnitPrice = Guard.NonNegative(unitPrice, nameof(unitPrice), 10_000m);
        var validatedOccurredAt = Guard.NotFuture(occurredAt, now, TimeSpan.FromMinutes(5), nameof(occurredAt));

        if (validatedOccurredAt < new DateTimeOffset(2000, 1, 1, 0, 0, 0, TimeSpan.Zero))
        {
            throw new DomainException("OccurredAt is outside the supported transaction window.");
        }

        return new FuelTransaction
        {
            Id = Guid.NewGuid(),
            SiteId = site.Id,
            DispenserId = dispenser.Id,
            ProductId = product.Id,
            ExternalId = Guard.NotBlank(externalId, nameof(externalId), 120),
            OccurredAt = validatedOccurredAt.ToUniversalTime(),
            Litres = validatedLitres,
            UnitPrice = validatedUnitPrice,
            Total = decimal.Round(validatedLitres * validatedUnitPrice, 2, MidpointRounding.AwayFromZero),
            PaymentMethod = paymentMethod,
            Status = TransactionStatus.Recorded,
            CreatedAt = now.ToUniversalTime(),
            Version = 1
        };
    }

    public void Void()
    {
        if (Status == TransactionStatus.Voided)
        {
            throw new DomainException("The transaction is already voided.");
        }

        Status = TransactionStatus.Voided;
    }
}
