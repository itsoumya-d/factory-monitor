using FuelIntegrationHub.Domain.Common;

namespace FuelIntegrationHub.Domain.Sites;

public sealed class Site
{
    private readonly List<Dispensers.Dispenser> _dispensers = [];

    private Site()
    {
    }

    public Guid Id { get; private set; }

    public string Code { get; private set; } = string.Empty;

    public string Name { get; private set; } = string.Empty;

    public string Region { get; private set; } = string.Empty;

    public string Currency { get; private set; } = "NZD";

    public bool IsActive { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public int Version { get; private set; }

    public IReadOnlyCollection<Dispensers.Dispenser> Dispensers => _dispensers;

    public static Site Create(string code, string name, string region, string currency, DateTimeOffset now)
    {
        return new Site
        {
            Id = Guid.NewGuid(),
            Code = Guard.NotBlank(code, nameof(code), 32).ToUpperInvariant(),
            Name = Guard.NotBlank(name, nameof(name), 200),
            Region = Guard.NotBlank(region, nameof(region), 80),
            Currency = Guard.NotBlank(currency, nameof(currency), 3).ToUpperInvariant(),
            IsActive = true,
            CreatedAt = now.ToUniversalTime(),
            Version = 1
        };
    }

    public void Rename(string name, string region)
    {
        Name = Guard.NotBlank(name, nameof(name), 200);
        Region = Guard.NotBlank(region, nameof(region), 80);
    }

    public void Deactivate()
    {
        IsActive = false;
    }

    public void Activate()
    {
        IsActive = true;
    }
}
