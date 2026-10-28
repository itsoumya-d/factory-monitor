@description('Azure region for all resources.')
param location string = resourceGroup().location

@description('Short prefix used in resource names.')
@minLength(3)
@maxLength(12)
param namePrefix string = 'fuelhub'

@description('Container image to deploy, for example myregistry.azurecr.io/fuel-integration-hub:latest.')
param containerImage string

@description('API key required by integration clients.')
@secure()
param apiKey string

@description('Shared secret used to verify inbound webhook signatures.')
@secure()
param webhookSecret string

@description('Minimum number of API replicas.')
@minValue(1)
@maxValue(1)
param minReplicas int = 1

@description('Maximum number of API replicas. Keep at 1 while SQLite is the backing store.')
@minValue(1)
@maxValue(1)
param maxReplicas int = 1

var suffix = uniqueString(resourceGroup().id)
var storageAccountName = toLower(take('${namePrefix}${suffix}', 24))
var fileShareName = 'fueldata'

resource logAnalytics 'Microsoft.OperationalInsights/workspaces@2023-09-01' = {
  name: '${namePrefix}-logs-${suffix}'
  location: location
  properties: {
    sku: {
      name: 'PerGB2018'
    }
    retentionInDays: 30
  }
}

resource storageAccount 'Microsoft.Storage/storageAccounts@2023-05-01' = {
  name: storageAccountName
  location: location
  kind: 'StorageV2'
  sku: {
    name: 'Standard_LRS'
  }
  properties: {
    minimumTlsVersion: 'TLS1_2'
    allowBlobPublicAccess: false
    supportsHttpsTrafficOnly: true
  }
}

resource fileShare 'Microsoft.Storage/storageAccounts/fileServices/shares@2023-05-01' = {
  parent: storageAccount
  name: 'default/${fileShareName}'
  properties: {
    shareQuota: 5
  }
}

resource managedEnvironment 'Microsoft.App/managedEnvironments@2024-03-01' = {
  name: '${namePrefix}-env-${suffix}'
  location: location
  properties: {
    appLogsConfiguration: {
      destination: 'log-analytics'
      logAnalyticsConfiguration: {
        customerId: logAnalytics.properties.customerId
        sharedKey: logAnalytics.listKeys().primarySharedKey
      }
    }
  }
}

resource environmentStorage 'Microsoft.App/managedEnvironments/storages@2024-03-01' = {
  parent: managedEnvironment
  name: 'fueldata'
  properties: {
    azureFile: {
      accountName: storageAccount.name
      accountKey: storageAccount.listKeys().keys[0].value
      shareName: fileShareName
      accessMode: 'ReadWrite'
    }
  }
}

resource containerApp 'Microsoft.App/containerApps@2024-03-01' = {
  name: '${namePrefix}-api-${suffix}'
  location: location
  identity: {
    type: 'SystemAssigned'
  }
  properties: {
    managedEnvironmentId: managedEnvironment.id
    configuration: {
      activeRevisionsMode: 'Single'
      ingress: {
        external: true
        targetPort: 8080
        transport: 'auto'
        allowInsecure: false
      }
      secrets: [
        {
          name: 'api-key'
          value: apiKey
        }
        {
          name: 'webhook-secret'
          value: webhookSecret
        }
      ]
    }
    template: {
      containers: [
        {
          name: 'api'
          image: containerImage
          resources: {
            cpu: json('0.5')
            memory: '1Gi'
          }
          env: [
            {
              name: 'ASPNETCORE_ENVIRONMENT'
              value: 'Production'
            }
            {
              name: 'ConnectionStrings__Default'
              value: 'Data Source=/app/data/fuel-integration.db'
            }
            {
              name: 'Database__EnsureCreated'
              value: 'true'
            }
            {
              name: 'Database__Seed'
              value: 'true'
            }
            {
              name: 'Security__ApiKey'
              secretRef: 'api-key'
            }
            {
              name: 'Security__WebhookSecret'
              secretRef: 'webhook-secret'
            }
            {
              name: 'Outbox__Enabled'
              value: 'true'
            }
            {
              name: 'Erp__Mode'
              value: 'Simulated'
            }
          ]
          probes: [
            {
              type: 'liveness'
              httpGet: {
                path: '/health'
                port: 8080
              }
              initialDelaySeconds: 10
              periodSeconds: 30
            }
            {
              type: 'readiness'
              httpGet: {
                path: '/health/ready'
                port: 8080
              }
              initialDelaySeconds: 5
              periodSeconds: 10
            }
          ]
          volumeMounts: [
            {
              volumeName: 'fueldata'
              mountPath: '/app/data'
            }
          ]
        }
      ]
      volumes: [
        {
          name: 'fueldata'
          storageType: 'AzureFile'
          storageName: 'fueldata'
        }
      ]
      scale: {
        minReplicas: minReplicas
        maxReplicas: maxReplicas
      }
    }
  }
  dependsOn: [
    environmentStorage
  ]
}

output apiUrl string = 'https://${containerApp.properties.configuration.ingress.fqdn}'
output apiResourceName string = containerApp.name
output logAnalyticsWorkspace string = logAnalytics.name
