FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
WORKDIR /src

COPY global.json Directory.Build.props FuelIntegrationHub.slnx ./
COPY src/FuelIntegrationHub.Domain/FuelIntegrationHub.Domain.csproj src/FuelIntegrationHub.Domain/
COPY src/FuelIntegrationHub.Infrastructure/FuelIntegrationHub.Infrastructure.csproj src/FuelIntegrationHub.Infrastructure/
COPY src/FuelIntegrationHub.Api/FuelIntegrationHub.Api.csproj src/FuelIntegrationHub.Api/
COPY tests/FuelIntegrationHub.Domain.Tests/FuelIntegrationHub.Domain.Tests.csproj tests/FuelIntegrationHub.Domain.Tests/
COPY tests/FuelIntegrationHub.Api.Tests/FuelIntegrationHub.Api.Tests.csproj tests/FuelIntegrationHub.Api.Tests/
COPY bench/FuelIntegrationHub.Benchmarks/FuelIntegrationHub.Benchmarks.csproj bench/FuelIntegrationHub.Benchmarks/

RUN dotnet restore FuelIntegrationHub.slnx
COPY . .
RUN dotnet publish src/FuelIntegrationHub.Api/FuelIntegrationHub.Api.csproj -c Release -o /app/publish /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS runtime
WORKDIR /app
COPY --from=build /app/publish .

ENV ASPNETCORE_URLS=http://+:8080
EXPOSE 8080

ENTRYPOINT ["dotnet", "FuelIntegrationHub.Api.dll"]
