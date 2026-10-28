using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Infrastructure.Persistence;

public static class DatabaseSeeder
{
    public static async Task SeedAsync(FuelDbContext db, DateTimeOffset now, CancellationToken cancellationToken = default)
    {
        if (await db.Sites.AnyAsync(cancellationToken))
        {
            return;
        }

        var auckland = Site.Create("AKL-01", "Auckland Airport Fuel Stop", "Auckland", "NZD", now);
        var hamilton = Site.Create("HLZ-01", "Hamilton Central Fuel Stop", "Waikato", "NZD", now);

        var petrol91 = Product.Create("91", "Regular Petrol 91", "L", now);
        var diesel = Product.Create("DSL", "Diesel", "L", now);

        var aucklandP91 = Dispenser.Create(auckland, petrol91, "D1", now);
        var aucklandDiesel = Dispenser.Create(auckland, diesel, "D2", now);
        var hamiltonP91 = Dispenser.Create(hamilton, petrol91, "D1", now);

        await db.Sites.AddRangeAsync([auckland, hamilton], cancellationToken);
        await db.Products.AddRangeAsync([petrol91, diesel], cancellationToken);
        await db.Dispensers.AddRangeAsync([aucklandP91, aucklandDiesel, hamiltonP91], cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
    }
}
