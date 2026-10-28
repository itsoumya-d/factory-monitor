using FuelIntegrationHub.Domain.Outbox;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace FuelIntegrationHub.Infrastructure.Persistence.Configurations;

internal sealed class OutboxMessageConfiguration : IEntityTypeConfiguration<OutboxMessage>
{
    public void Configure(EntityTypeBuilder<OutboxMessage> builder)
    {
        builder.ToTable("outbox_messages");
        builder.HasKey(message => message.Id);

        builder.Property(message => message.Id).HasColumnName("id");
        builder.Property(message => message.Type).HasColumnName("type").HasMaxLength(120).IsRequired();
        builder.Property(message => message.Payload).HasColumnName("payload").IsRequired();
        builder.Property(message => message.IdempotencyKey)
            .HasColumnName("idempotency_key")
            .HasMaxLength(200)
            .IsRequired();
        builder.Property(message => message.Status)
            .HasColumnName("status")
            .HasConversion<string>()
            .HasMaxLength(32)
            .IsRequired();
        builder.Property(message => message.Attempts).HasColumnName("attempts").IsRequired();
        builder.Property(message => message.OccurredAt).HasColumnName("occurred_at").IsRequired();
        builder.Property(message => message.NextAttemptAt).HasColumnName("next_attempt_at").IsRequired();
        builder.Property(message => message.ProcessedAt).HasColumnName("processed_at");
        builder.Property(message => message.LastError).HasColumnName("last_error").HasMaxLength(2_000);

        builder.HasIndex(message => message.IdempotencyKey)
            .IsUnique()
            .HasDatabaseName("ux_outbox_messages_idempotency_key");

        builder.HasIndex(message => new { message.Status, message.NextAttemptAt })
            .HasDatabaseName("ix_outbox_messages_status_next_attempt");
    }
}
