using FuelIntegrationHub.Api.Endpoints;
using FuelIntegrationHub.Api.ErrorHandling;
using FuelIntegrationHub.Api.Security;
using FuelIntegrationHub.Infrastructure;
using FuelIntegrationHub.Infrastructure.Persistence;
using Microsoft.AspNetCore.Authentication;
using Microsoft.EntityFrameworkCore;

var builder = WebApplication.CreateBuilder(args);

builder.Services.AddProblemDetails();
builder.Services.AddExceptionHandler<GlobalExceptionHandler>();
builder.Services.AddOpenApi();
builder.Services.AddHealthChecks();

builder.Services.AddOptions<SecurityOptions>()
    .BindConfiguration(SecurityOptions.SectionName)
    .Validate(
        options => options.ApiKey.Length >= 16,
        "Security:ApiKey must be at least 16 characters.")
    .Validate(
        options => options.WebhookSecret.Length >= 16,
        "Security:WebhookSecret must be at least 16 characters.")
    .Validate(
        options => options.WebhookMaxSkewSeconds is >= 30 and <= 3600,
        "Security:WebhookMaxSkewSeconds must be between 30 and 3600.")
    .ValidateOnStart();

builder.Services
    .AddAuthentication(ApiKeyDefaults.AuthenticationScheme)
    .AddScheme<AuthenticationSchemeOptions, ApiKeyAuthenticationHandler>(
        ApiKeyDefaults.AuthenticationScheme,
        _ => { });

builder.Services.AddAuthorization();

var connectionString = builder.Configuration.GetConnectionString("Default")
    ?? "Data Source=fuel-integration.db";

builder.Services.AddDbContext<FuelDbContext>(options =>
{
    options.UseSqlite(connectionString);
    if (builder.Configuration.GetValue("Database:DetailedErrors", false))
    {
        options.EnableDetailedErrors();
    }
});

builder.Services.AddInfrastructure(builder.Configuration);

var app = builder.Build();

app.UseExceptionHandler();
app.UseAuthentication();
app.UseAuthorization();
app.UseMiddleware<WebhookSignatureMiddleware>();

app.MapOpenApi();
app.MapHealthChecks("/health");
app.MapHealthChecks("/health/ready");
app.MapGet("/", () => Results.Redirect("/openapi/v1.json"));

app.MapTransactionEndpoints();
app.MapReconciliationEndpoints();
app.MapOpsEndpoints();

await DatabaseInitializer.InitializeAsync(app.Services, app.Configuration);

app.Run();

public partial class Program;
