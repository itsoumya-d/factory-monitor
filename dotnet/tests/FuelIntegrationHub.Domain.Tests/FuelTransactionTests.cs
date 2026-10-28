using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Transactions;

namespace FuelIntegrationHub.Domain.Tests;

public sealed class FuelTransactionTests
{
    [Fact]
    public void RecordCalculatesTotalToTwoDecimalPlaces()
    {
        var site = TestData.CreateSite();
        var product = TestData.CreatePetrol();
        var dispenser = TestData.CreateDispenser(site, product);

        var transaction = FuelTransaction.Record(
            site,
            dispenser,
            product,
            "TX-1001",
            TestData.Now.AddMinutes(-1),
            42.5m,
            2.349m,
            PaymentMethod.FleetCard,
            TestData.Now);

        Assert.Equal(99.83m, transaction.Total);
        Assert.Equal(TransactionStatus.Recorded, transaction.Status);
        Assert.Equal("TX-1001", transaction.ExternalId);
        Assert.Equal(1, transaction.Version);
    }

    [Fact]
    public void RecordRejectsNonPositiveLitres()
    {
        var site = TestData.CreateSite();
        var product = TestData.CreatePetrol();
        var dispenser = TestData.CreateDispenser(site, product);

        var exception = Assert.Throws<DomainException>(() => FuelTransaction.Record(
            site,
            dispenser,
            product,
            "TX-1002",
            TestData.Now,
            0m,
            2.30m,
            PaymentMethod.Card,
            TestData.Now));

        Assert.Contains("litres", exception.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RecordRejectsFutureTimestampBeyondTolerance()
    {
        var site = TestData.CreateSite();
        var product = TestData.CreatePetrol();
        var dispenser = TestData.CreateDispenser(site, product);

        Assert.Throws<DomainException>(() => FuelTransaction.Record(
            site,
            dispenser,
            product,
            "TX-1003",
            TestData.Now.AddMinutes(10),
            10m,
            2.30m,
            PaymentMethod.Card,
            TestData.Now));
    }

    [Fact]
    public void RecordRejectsDispenserProductMismatch()
    {
        var site = TestData.CreateSite();
        var petrol = TestData.CreatePetrol();
        var diesel = Product.Create("DSL", "Diesel", "L", TestData.Now);
        var dispenser = TestData.CreateDispenser(site, petrol);

        Assert.Throws<DomainException>(() => FuelTransaction.Record(
            site,
            dispenser,
            diesel,
            "TX-1004",
            TestData.Now,
            10m,
            2.30m,
            PaymentMethod.Card,
            TestData.Now));
    }

    [Fact]
    public void RecordRejectsInactiveSite()
    {
        var site = TestData.CreateSite();
        var product = TestData.CreatePetrol();
        var dispenser = TestData.CreateDispenser(site, product);
        site.Deactivate();

        Assert.Throws<DomainException>(() => FuelTransaction.Record(
            site,
            dispenser,
            product,
            "TX-1005",
            TestData.Now,
            10m,
            2.30m,
            PaymentMethod.Card,
            TestData.Now));
    }

    [Fact]
    public void VoidCanOnlyBeAppliedOnce()
    {
        var site = TestData.CreateSite();
        var product = TestData.CreatePetrol();
        var dispenser = TestData.CreateDispenser(site, product);
        var transaction = FuelTransaction.Record(
            site,
            dispenser,
            product,
            "TX-1006",
            TestData.Now,
            10m,
            2.30m,
            PaymentMethod.Card,
            TestData.Now);

        transaction.Void();

        Assert.Equal(TransactionStatus.Voided, transaction.Status);
        Assert.Throws<DomainException>(transaction.Void);
    }
}
