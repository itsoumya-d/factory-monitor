using FuelIntegrationHub.Infrastructure.Integrations;
using FuelIntegrationHub.Infrastructure.Ingestion;
using FuelIntegrationHub.Infrastructure.Outbox;
using FuelIntegrationHub.Infrastructure.Persistence;
using FuelIntegrationHub.Infrastructure.Reconciliation;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;

namespace FuelIntegrationHub.Infrastructure;

public static class InfrastructureServiceCollectionExtensions
{
    public static IServiceCollection AddInfrastructure(
        this IServiceCollection services,
        IConfiguration configuration)
    {
        services.AddSingleton(TimeProvider.System);
        services.AddScoped<FuelIngestionService>();
        services.AddScoped<FuelReconciliationService>();
        services.AddScoped<OutboxProcessor>();
        services.AddSingleton<InMemoryErpSink>();
        services.Configure<OutboxOptions>(configuration.GetSection(OutboxOptions.SectionName));

        var erpMode = configuration.GetValue("Erp:Mode", "Simulated");
        if (string.Equals(erpMode, "Http", StringComparison.OrdinalIgnoreCase))
        {
            services.AddHttpClient<IErpClient, HttpErpClient>(client =>
            {
                client.BaseAddress = new Uri(
                    configuration.GetValue("Erp:BaseUrl", "https://erp.example.com/")!,
                    UriKind.Absolute);
                client.Timeout = TimeSpan.FromSeconds(configuration.GetValue("Erp:TimeoutSeconds", 30));
                client.DefaultRequestHeaders.Add("Accept", "application/json");
            });
        }
        else
        {
            services.AddSingleton<IErpClient, SimulatedErpClient>();
        }

        services.AddHealthChecks()
            .AddCheck<DatabaseHealthCheck>("database");

        services.AddHostedService<OutboxDispatcher>();

        return services;
    }
}
