using FuelIntegrationHub.Infrastructure.Logging;
using Microsoft.Extensions.Logging;

namespace FuelIntegrationHub.Infrastructure.Integrations;

public sealed class SimulatedErpClient(InMemoryErpSink sink, ILogger<SimulatedErpClient> logger) : IErpClient
{
    public Task SendTransactionAsync(ErpTransactionEvent transactionEvent, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        sink.Add(transactionEvent);
        Log.ErpAccepted(logger, transactionEvent.EventId, transactionEvent.SiteCode, transactionEvent.Litres);

        return Task.CompletedTask;
    }
}
