namespace FuelIntegrationHub.Infrastructure.Integrations;

public interface IErpClient
{
    Task SendTransactionAsync(ErpTransactionEvent transactionEvent, CancellationToken cancellationToken);
}
