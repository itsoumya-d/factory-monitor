namespace FuelIntegrationHub.Api.Security;

public sealed class SecurityOptions
{
    public const string SectionName = "Security";

    public string ApiKey { get; init; } = string.Empty;

    public string WebhookSecret { get; init; } = string.Empty;

    public int WebhookMaxSkewSeconds { get; init; } = 300;
}
