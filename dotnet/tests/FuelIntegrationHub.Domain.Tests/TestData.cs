using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;

namespace FuelIntegrationHub.Domain.Tests;

internal static class TestData
{
    public static readonly DateTimeOffset Now = new(2026, 10, 28, 12, 0, 0, TimeSpan.Zero);

    public static Site CreateSite(string code = "AKL-01")
    {
        return Site.Create(code, "Auckland Airport Fuel Stop", "Auckland", "NZD", Now);
    }

    public static Product CreatePetrol()
    {
        return Product.Create("91", "Regular Petrol 91", "L", Now);
    }

    public static Dispenser CreateDispenser(Site site, Product product, string code = "D1")
    {
        return Dispenser.Create(site, product, code, Now);
    }
}
