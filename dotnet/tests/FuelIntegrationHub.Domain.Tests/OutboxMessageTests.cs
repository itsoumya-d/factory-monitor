using FuelIntegrationHub.Domain.Outbox;

namespace FuelIntegrationHub.Domain.Tests;

public sealed class OutboxMessageTests
{
    [Fact]
    public void EnqueueStartsPendingAndReady()
    {
        var message = OutboxMessage.Enqueue(
            "fuel-transaction.recorded",
            "{}",
            "tx:AKL-01:TX-1",
            TestData.Now);

        Assert.Equal(OutboxStatus.Pending, message.Status);
        Assert.Equal(0, message.Attempts);
        Assert.Equal(TestData.Now, message.NextAttemptAt);
    }

    [Fact]
    public void MarkFailedAppliesExponentialBackoff()
    {
        var message = OutboxMessage.Enqueue("type", "{}", "key", TestData.Now);

        message.MarkFailed("timeout", TestData.Now, maxAttempts: 5);

        Assert.Equal(OutboxStatus.Pending, message.Status);
        Assert.Equal(1, message.Attempts);
        Assert.Equal(TestData.Now.AddSeconds(2), message.NextAttemptAt);
    }

    [Fact]
    public void MarkFailedDeadLettersAtMaxAttempts()
    {
        var message = OutboxMessage.Enqueue("type", "{}", "key", TestData.Now);

        message.MarkFailed("timeout", TestData.Now, maxAttempts: 2);
        message.MarkFailed("timeout", TestData.Now, maxAttempts: 2);

        Assert.Equal(OutboxStatus.DeadLetter, message.Status);
        Assert.Equal(2, message.Attempts);
        Assert.Null(message.ProcessedAt);
    }

    [Fact]
    public void MarkSentRecordsProcessedTimestamp()
    {
        var message = OutboxMessage.Enqueue("type", "{}", "key", TestData.Now);
        message.MarkProcessing();

        message.MarkSent(TestData.Now.AddSeconds(5));

        Assert.Equal(OutboxStatus.Sent, message.Status);
        Assert.Equal(TestData.Now.AddSeconds(5), message.ProcessedAt);
    }

    [Fact]
    public void RequeueResetsFailureState()
    {
        var message = OutboxMessage.Enqueue("type", "{}", "key", TestData.Now);
        message.MarkFailed("timeout", TestData.Now, maxAttempts: 1);
        Assert.Equal(OutboxStatus.DeadLetter, message.Status);

        message.Requeue(TestData.Now.AddMinutes(1));

        Assert.Equal(OutboxStatus.Pending, message.Status);
        Assert.Equal(0, message.Attempts);
        Assert.Null(message.LastError);
        Assert.Equal(TestData.Now.AddMinutes(1), message.NextAttemptAt);
    }
}
