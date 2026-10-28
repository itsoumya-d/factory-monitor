# Azure deployment

The API deploys to Azure Container Apps with Azure Files as persistent storage for the SQLite database. Log Analytics captures container logs. This is a working, low-cost deployment for evaluation; move the persistence layer to Azure SQL or PostgreSQL before running multiple replicas.

## Build and push the image

From the `dotnet/` directory:

```bash
az acr build \
  --registry <your-acr-name> \
  --image fuel-integration-hub:latest \
  --file src/FuelIntegrationHub.Api/Dockerfile \
  .
```

## Deploy

```bash
az group create --name rg-fuel-integration --location australiaeast

az deployment group create \
  --resource-group rg-fuel-integration \
  --template-file deploy/main.bicep \
  --parameters \
    containerImage=<your-acr-name>.azurecr.io/fuel-integration-hub:latest \
    apiKey='<at-least-16-random-characters>' \
    webhookSecret='<at-least-16-random-characters>'
```

The deployment outputs the public HTTPS URL. Verify it with:

```bash
curl -fsS https://<api-url>/health
```

## Operations

```bash
az containerapp logs show \
  --name <apiResourceName> \
  --resource-group rg-fuel-integration \
  --follow
```

The Bicep template intentionally caps the app at one replica because SQLite on Azure Files is a single-writer store. The outbox worker and API share that database. For horizontal scale, replace SQLite with Azure SQL/PostgreSQL and keep the same EF Core model, indexes, and outbox contract.
