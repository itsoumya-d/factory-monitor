using System.Net.Http.Json;
using FuelIntegrationHub.Infrastructure.Logging;
using Microsoft.Extensions.Logging;

namespace FuelIntegrationHub.Infrastructure.Integrations;

public sealed class HttpErpClient(HttpClient httpClient, ILogger<HttpErpClient> logger) : IErpClient
{
    public async Task SendTransactionAsync(ErpTransactionEvent transactionEvent, CancellationToken cancellationToken)
    {
        using var response = await httpClient.PostAsJsonAsync(
            "api/v1/fuel-transactions",
            transactionEvent,
            cancellationToken);

        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync(cancellationToken);
            Log.ErpRejected(logger, transactionEvent.EventId, (int)response.StatusCode, body);

            response.EnsureSuccessStatusCode();
        }
    }
}
