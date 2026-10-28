using System.Collections.Concurrent;

namespace FuelIntegrationHub.Infrastructure.Integrations;

public sealed class InMemoryErpSink
{
    private readonly ConcurrentQueue<ErpTransactionEvent> _events = new();

    public IReadOnlyCollection<ErpTransactionEvent> Events => _events.ToArray();

    public void Add(ErpTransactionEvent transactionEvent)
    {
        _events.Enqueue(transactionEvent);
    }
}
