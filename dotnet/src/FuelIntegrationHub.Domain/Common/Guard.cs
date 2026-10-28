namespace FuelIntegrationHub.Domain.Common;

internal static class Guard
{
    public static string NotBlank(string? value, string name, int maxLength)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            throw new DomainException($"{name} is required.");
        }

        var trimmed = value.Trim();
        if (trimmed.Length > maxLength)
        {
            throw new DomainException($"{name} must be {maxLength} characters or fewer.");
        }

        return trimmed;
    }

    public static decimal Positive(decimal value, string name, decimal max)
    {
        if (value <= 0 || value > max)
        {
            throw new DomainException($"{name} must be greater than zero and no more than {max}.");
        }

        return value;
    }

    public static decimal NonNegative(decimal value, string name, decimal max)
    {
        if (value < 0 || value > max)
        {
            throw new DomainException($"{name} must be between zero and {max}.");
        }

        return value;
    }

    public static int Between(int value, string name, int min, int max)
    {
        if (value < min || value > max)
        {
            throw new DomainException($"{name} must be between {min} and {max}.");
        }

        return value;
    }

    public static DateTimeOffset NotFuture(DateTimeOffset value, DateTimeOffset now, TimeSpan tolerance, string name)
    {
        if (value > now.Add(tolerance))
        {
            throw new DomainException($"{name} cannot be more than {tolerance.TotalMinutes} minutes in the future.");
        }

        return value;
    }
}
