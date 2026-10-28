namespace FuelIntegrationHub.Domain.Outbox;

public enum OutboxStatus
{
    Pending = 1,
    Processing = 2,
    Sent = 3,
    DeadLetter = 4
}
