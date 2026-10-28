using System.Diagnostics;
using System.Globalization;
using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;
using FuelIntegrationHub.Domain.Transactions;
using FuelIntegrationHub.Infrastructure.Ingestion;
using FuelIntegrationHub.Infrastructure.Persistence;
using FuelIntegrationHub.Infrastructure.Reconciliation;
using Microsoft.EntityFrameworkCore;

var rows = args.Length > 0 && int.TryParse(args[0], NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
    ? parsed
    : 50_000;

var databasePath = Path.Combine(Path.GetTempPath(), $"fuel-bench-{Guid.NewGuid():N}.db");
var options = new DbContextOptionsBuilder<FuelDbContext>()
    .UseSqlite($"Data Source={databasePath}")
    .Options;

await using var db = new FuelDbContext(options);
await db.Database.EnsureCreatedAsync();

var now = TimeProvider.System.GetUtcNow();
var site = Site.Create("BENCH-01", "Benchmark Fuel Stop", "Waikato", "NZD", now);
var petrol = Product.Create("91", "Regular Petrol 91", "L", now);
var dispenser = Dispenser.Create(site, petrol, "D1", now);

db.Sites.Add(site);
db.Products.Add(petrol);
db.Dispensers.Add(dispenser);
await db.SaveChangesAsync();

var ingestion = new FuelIngestionService(db, TimeProvider.System);
var reconciliation = new FuelReconciliationService(db, TimeProvider.System);
const int batchSize = 500;
var batches = (rows + batchSize - 1) / batchSize;

var ingestWatch = Stopwatch.StartNew();
var inserted = 0;
for (var batch = 0; batch < batches; batch++)
{
    var commands = new List<IngestTransactionCommand>(batchSize);
    for (var offset = 0; offset < batchSize && inserted < rows; offset++, inserted++)
    {
        commands.Add(new IngestTransactionCommand(
            $"BENCH-{inserted:D8}",
            now.AddHours(-1).AddMilliseconds(inserted),
            dispenser.Code,
            petrol.Code,
            40m,
            2.45m,
            PaymentMethod.FleetCard));
    }

    var results = await ingestion.IngestBatchAsync(site.Code, commands, CancellationToken.None);
    if (results.Any(result => result.Status != "recorded"))
    {
        throw new InvalidOperationException("Benchmark ingestion produced a rejected transaction.");
    }
}

ingestWatch.Stop();

var periodStart = now.AddHours(-2);
var periodEnd = now.AddHours(1);
var siteLitres = await db.FuelTransactions
    .Where(transaction => transaction.OccurredAt >= periodStart && transaction.OccurredAt < periodEnd)
    .SumAsync(transaction => (decimal?)transaction.Litres) ?? 0m;

var reconciliationWatch = Stopwatch.StartNew();
var run = await reconciliation.RunAsync(
    site.Code,
    periodStart,
    periodEnd,
    erpLitres: decimal.Round(siteLitres * 0.999m, 3),
    thresholdPercent: 0.5m,
    CancellationToken.None);
reconciliationWatch.Stop();

var queryWatch = Stopwatch.StartNew();
var page = await db.FuelTransactions
    .AsNoTracking()
    .Where(transaction => transaction.SiteId == site.Id)
    .OrderByDescending(transaction => transaction.OccurredAt)
    .Skip(100 * 50)
    .Take(50)
    .Select(transaction => new
    {
        transaction.Id,
        transaction.ExternalId,
        transaction.Total,
        DispenserCode = transaction.Dispenser.Code
    })
    .ToListAsync();
queryWatch.Stop();

var outboxCount = await db.OutboxMessages.CountAsync();

Console.WriteLine("## Benchmark results");
Console.WriteLine();
Console.WriteLine($"- Environment: .NET {Environment.Version}, {System.Runtime.InteropServices.RuntimeInformation.OSDescription}, {System.Runtime.InteropServices.RuntimeInformation.ProcessArchitecture}");
Console.WriteLine($"- Dataset: {rows.ToString("N0", CultureInfo.InvariantCulture)} transactions, {outboxCount.ToString("N0", CultureInfo.InvariantCulture)} outbox messages, SQLite file database");
Console.WriteLine();
Console.WriteLine("| Operation | Work | Elapsed | Throughput |");
Console.WriteLine("|---|---:|---:|---:|");
Console.WriteLine($"| Batch ingestion | {rows.ToString("N0", CultureInfo.InvariantCulture)} transactions | {ingestWatch.Elapsed.TotalMilliseconds.ToString("N0", CultureInfo.InvariantCulture)} ms | {(rows / ingestWatch.Elapsed.TotalSeconds).ToString("N0", CultureInfo.InvariantCulture)} tx/s |");
Console.WriteLine($"| Reconciliation | {rows.ToString("N0", CultureInfo.InvariantCulture)} rows aggregated | {reconciliationWatch.Elapsed.TotalMilliseconds.ToString("N0", CultureInfo.InvariantCulture)} ms | {(rows / reconciliationWatch.Elapsed.TotalSeconds).ToString("N0", CultureInfo.InvariantCulture)} rows/s |");
Console.WriteLine($"| Paged query | {page.Count} rows (page 101) | {queryWatch.Elapsed.TotalMilliseconds.ToString("N2", CultureInfo.InvariantCulture)} ms | {(queryWatch.Elapsed.TotalMilliseconds / Math.Max(1, page.Count)).ToString("N3", CultureInfo.InvariantCulture)} ms/row |");
Console.WriteLine();
Console.WriteLine($"Reconciliation status: {run.Status} ({run.VariancePercent.ToString("N4", CultureInfo.InvariantCulture)}% variance, threshold {run.ThresholdPercent.ToString("N4", CultureInfo.InvariantCulture)}%)");

await db.Database.CloseConnectionAsync();
File.Delete(databasePath);
