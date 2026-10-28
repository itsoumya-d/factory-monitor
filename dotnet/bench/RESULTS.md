# Benchmark results

Command:

```bash
dotnet run --project bench/FuelIntegrationHub.Benchmarks -c Release -- 50000
```

Environment: .NET 10.0.14, macOS 26.5.1, Arm64, file-backed SQLite.

| Operation | Work | Elapsed | Throughput |
|---|---:|---:|---:|
| Batch ingestion | 50,000 transactions | 10,464 ms | 4,778 tx/s |
| Reconciliation | 50,000 rows aggregated | 158 ms | 316,884 rows/s |
| Paged query | 50 rows (page 101) | 10.72 ms | 0.214 ms/row |

Reconciliation status: Balanced, 0.1001% variance against a 0.5000% threshold.

These numbers measure the local engineering path (validation, EF Core persistence, outbox enqueue, aggregation, paging). They are not a production capacity plan.
