using System.Text.Json;
using FuelIntegrationHub.Domain.Common;
using FuelIntegrationHub.Domain.Dispensers;
using FuelIntegrationHub.Domain.Outbox;
using FuelIntegrationHub.Domain.Products;
using FuelIntegrationHub.Domain.Sites;
using FuelIntegrationHub.Domain.Transactions;
using FuelIntegrationHub.Infrastructure.Integrations;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace FuelIntegrationHub.Infrastructure.Ingestion;

public sealed class FuelIngestionService(FuelDbContext db, TimeProvider timeProvider)
{
    private const string TransactionRecordedEvent = "fuel-transaction.recorded";

    public async Task<IngestTransactionResult> IngestAsync(
        string siteCode,
        IngestTransactionCommand command,
        CancellationToken cancellationToken)
    {
        var site = await GetSiteAsync(siteCode, cancellationToken);
        var dispenser = await GetDispenserAsync(site.Id, command.DispenserCode, cancellationToken);
        var product = ValidateProduct(dispenser, command.ProductCode);

        var existing = await FindExistingAsync(site.Id, command.ExternalId, cancellationToken);
        if (existing is not null)
        {
            return ToDuplicateResult(existing);
        }

        var now = timeProvider.GetUtcNow();
        var transaction = FuelTransaction.Record(
            site,
            dispenser,
            product,
            command.ExternalId,
            command.OccurredAt,
            command.Litres,
            command.UnitPrice,
            command.PaymentMethod,
            now);

        db.FuelTransactions.Add(transaction);
        db.OutboxMessages.Add(CreateOutboxMessage(site, transaction, now));

        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateException)
        {
            db.ChangeTracker.Clear();
            existing = await FindExistingAsync(site.Id, command.ExternalId, cancellationToken);
            if (existing is not null)
            {
                return ToDuplicateResult(existing);
            }

            throw;
        }

        return new IngestTransactionResult(
            transaction.Id,
            transaction.ExternalId,
            "recorded",
            transaction.Total,
            Duplicate: false);
    }

    public async Task<IReadOnlyList<BatchIngestItemResult>> IngestBatchAsync(
        string siteCode,
        IReadOnlyList<IngestTransactionCommand> commands,
        CancellationToken cancellationToken)
    {
        var site = await GetSiteAsync(siteCode, cancellationToken);
        var dispensers = await db.Dispensers
            .Include(dispenser => dispenser.Product)
            .Where(dispenser => dispenser.SiteId == site.Id)
            .ToDictionaryAsync(dispenser => dispenser.Code, StringComparer.OrdinalIgnoreCase, cancellationToken);

        var externalIds = commands
            .Select(command => command.ExternalId)
            .Where(externalId => !string.IsNullOrWhiteSpace(externalId))
            .Distinct(StringComparer.Ordinal)
            .ToArray();

        var existingIds = await db.FuelTransactions
            .AsNoTracking()
            .Where(transaction => transaction.SiteId == site.Id && externalIds.Contains(transaction.ExternalId))
            .Select(transaction => transaction.ExternalId)
            .ToHashSetAsync(StringComparer.Ordinal, cancellationToken);

        var now = timeProvider.GetUtcNow();
        var results = new List<BatchIngestItemResult>(commands.Count);

        for (var index = 0; index < commands.Count; index++)
        {
            var command = commands[index];

            if (string.IsNullOrWhiteSpace(command.ExternalId))
            {
                results.Add(Rejected(index, command.ExternalId, "ExternalId is required."));
                continue;
            }

            if (existingIds.Contains(command.ExternalId))
            {
                var existing = await FindExistingAsync(site.Id, command.ExternalId, cancellationToken);
                results.Add(existing is null
                    ? Rejected(index, command.ExternalId, "Duplicate external id.")
                    : new BatchIngestItemResult(index, command.ExternalId, "duplicate", existing.Id, existing.Total, null));
                continue;
            }

            if (!dispensers.TryGetValue(command.DispenserCode, out var dispenser))
            {
                results.Add(Rejected(index, command.ExternalId, $"Dispenser '{command.DispenserCode}' was not found."));
                continue;
            }

            if (!string.Equals(dispenser.Product.Code, command.ProductCode, StringComparison.OrdinalIgnoreCase))
            {
                results.Add(Rejected(index, command.ExternalId, $"Dispenser '{dispenser.Code}' is not configured for product '{command.ProductCode}'."));
                continue;
            }

            try
            {
                var transaction = FuelTransaction.Record(
                    site,
                    dispenser,
                    dispenser.Product,
                    command.ExternalId,
                    command.OccurredAt,
                    command.Litres,
                    command.UnitPrice,
                    command.PaymentMethod,
                    now);

                db.FuelTransactions.Add(transaction);
                db.OutboxMessages.Add(CreateOutboxMessage(site, transaction, now));
                results.Add(new BatchIngestItemResult(index, transaction.ExternalId, "recorded", transaction.Id, transaction.Total, null));
            }
            catch (DomainException exception)
            {
                results.Add(Rejected(index, command.ExternalId, exception.Message));
            }
        }

        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateException)
        {
            db.ChangeTracker.Clear();
            throw;
        }

        return results;
    }

    private async Task<Site> GetSiteAsync(string siteCode, CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(siteCode))
        {
            throw new DomainException("Site code is required.");
        }

        var normalizedCode = siteCode.Trim().ToUpperInvariant();
        return await db.Sites.SingleOrDefaultAsync(site => site.Code == normalizedCode, cancellationToken)
            ?? throw new KeyNotFoundException($"Site '{siteCode}' was not found.");
    }

    private async Task<Dispenser> GetDispenserAsync(
        Guid siteId,
        string dispenserCode,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(dispenserCode))
        {
            throw new DomainException("Dispenser code is required.");
        }

        var normalizedCode = dispenserCode.Trim().ToUpperInvariant();
        return await db.Dispensers
            .Include(dispenser => dispenser.Product)
            .SingleOrDefaultAsync(
                dispenser => dispenser.SiteId == siteId && dispenser.Code == normalizedCode,
                cancellationToken)
            ?? throw new DomainException($"Dispenser '{dispenserCode}' was not found for this site.");
    }

    private static Product ValidateProduct(Dispenser dispenser, string productCode)
    {
        if (string.IsNullOrWhiteSpace(productCode))
        {
            throw new DomainException("Product code is required.");
        }

        var normalizedCode = productCode.Trim().ToUpperInvariant();
        if (!string.Equals(dispenser.Product.Code, normalizedCode, StringComparison.Ordinal))
        {
            throw new DomainException(
                $"Dispenser '{dispenser.Code}' is not configured for product '{normalizedCode}'.");
        }

        return dispenser.Product;
    }

    private Task<FuelTransaction?> FindExistingAsync(
        Guid siteId,
        string externalId,
        CancellationToken cancellationToken)
    {
        return db.FuelTransactions
            .AsNoTracking()
            .SingleOrDefaultAsync(
                transaction => transaction.SiteId == siteId && transaction.ExternalId == externalId,
                cancellationToken);
    }

    private static OutboxMessage CreateOutboxMessage(Site site, FuelTransaction transaction, DateTimeOffset now)
    {
        var transactionEvent = new ErpTransactionEvent(
            transaction.Id.ToString("N"),
            site.Code,
            transaction.ExternalId,
            transaction.Litres,
            transaction.UnitPrice,
            transaction.Total,
            transaction.OccurredAt);

        return OutboxMessage.Enqueue(
            TransactionRecordedEvent,
            JsonSerializer.Serialize(transactionEvent),
            $"tx:{site.Code}:{transaction.ExternalId}",
            now);
    }

    private static IngestTransactionResult ToDuplicateResult(FuelTransaction transaction)
    {
        return new IngestTransactionResult(
            transaction.Id,
            transaction.ExternalId,
            "duplicate",
            transaction.Total,
            Duplicate: true);
    }

    private static BatchIngestItemResult Rejected(int index, string externalId, string error)
    {
        return new BatchIngestItemResult(index, externalId, "rejected", null, null, error);
    }
}
