using Microsoft.EntityFrameworkCore.Storage.ValueConversion;

namespace FuelIntegrationHub.Infrastructure.Persistence;

internal sealed class DateTimeOffsetToUnixMillisecondsConverter()
    : ValueConverter<DateTimeOffset, long>(
        value => value.ToUniversalTime().ToUnixTimeMilliseconds(),
        value => DateTimeOffset.FromUnixTimeMilliseconds(value));
