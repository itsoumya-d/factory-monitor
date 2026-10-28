namespace FuelIntegrationHub.Domain.Outbox;

public static class OutboxBackoff
{
    private static readonly TimeSpan MaxDelay = TimeSpan.FromMinutes(5);

    public static TimeSpan NextDelay(int attempts)
    {
        if (attempts <= 0)
        {
            return TimeSpan.Zero;
        }

        var exponent = Math.Min(attempts, 10);
        var seconds = Math.Min(MaxDelay.TotalSeconds, Math.Pow(2, exponent));
        return TimeSpan.FromSeconds(seconds);
    }
}
