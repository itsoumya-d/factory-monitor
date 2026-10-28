CREATE TABLE sites (
    id uniqueidentifier NOT NULL CONSTRAINT pk_sites PRIMARY KEY,
    code nvarchar(32) NOT NULL,
    name nvarchar(200) NOT NULL,
    region nvarchar(80) NOT NULL,
    currency nvarchar(3) NOT NULL,
    is_active bit NOT NULL,
    created_at datetimeoffset NOT NULL,
    version int NOT NULL CONSTRAINT df_sites_version DEFAULT 1,
    CONSTRAINT ux_sites_code UNIQUE (code)
);

CREATE TABLE products (
    id uniqueidentifier NOT NULL CONSTRAINT pk_products PRIMARY KEY,
    code nvarchar(32) NOT NULL,
    name nvarchar(120) NOT NULL,
    unit nvarchar(8) NOT NULL,
    is_active bit NOT NULL,
    created_at datetimeoffset NOT NULL,
    CONSTRAINT ux_products_code UNIQUE (code)
);

CREATE TABLE dispensers (
    id uniqueidentifier NOT NULL CONSTRAINT pk_dispensers PRIMARY KEY,
    site_id uniqueidentifier NOT NULL,
    product_id uniqueidentifier NOT NULL,
    code nvarchar(32) NOT NULL,
    is_active bit NOT NULL,
    created_at datetimeoffset NOT NULL,
    CONSTRAINT fk_dispensers_sites FOREIGN KEY (site_id) REFERENCES sites (id),
    CONSTRAINT fk_dispensers_products FOREIGN KEY (product_id) REFERENCES products (id),
    CONSTRAINT ux_dispensers_site_code UNIQUE (site_id, code)
);

CREATE TABLE fuel_transactions (
    id uniqueidentifier NOT NULL CONSTRAINT pk_fuel_transactions PRIMARY KEY,
    site_id uniqueidentifier NOT NULL,
    dispenser_id uniqueidentifier NOT NULL,
    product_id uniqueidentifier NOT NULL,
    external_id nvarchar(120) NOT NULL,
    occurred_at datetimeoffset NOT NULL,
    litres decimal(18, 3) NOT NULL,
    unit_price decimal(18, 4) NOT NULL,
    total decimal(18, 2) NOT NULL,
    payment_method nvarchar(32) NOT NULL,
    status nvarchar(32) NOT NULL,
    created_at datetimeoffset NOT NULL,
    version int NOT NULL CONSTRAINT df_fuel_transactions_version DEFAULT 1,
    CONSTRAINT fk_fuel_transactions_sites FOREIGN KEY (site_id) REFERENCES sites (id),
    CONSTRAINT fk_fuel_transactions_dispensers FOREIGN KEY (dispenser_id) REFERENCES dispensers (id),
    CONSTRAINT fk_fuel_transactions_products FOREIGN KEY (product_id) REFERENCES products (id),
    CONSTRAINT ux_fuel_transactions_site_external UNIQUE (site_id, external_id),
    CONSTRAINT ck_fuel_transactions_litres CHECK (litres > 0 AND litres <= 100000),
    CONSTRAINT ck_fuel_transactions_unit_price CHECK (unit_price >= 0 AND unit_price <= 10000),
    CONSTRAINT ck_fuel_transactions_total CHECK (total >= 0),
    CONSTRAINT ck_fuel_transactions_payment_method CHECK (payment_method IN ('Cash', 'Card', 'FleetCard', 'Account', 'Unknown')),
    CONSTRAINT ck_fuel_transactions_status CHECK (status IN ('Recorded', 'Voided'))
);

CREATE INDEX ix_fuel_transactions_site_occurred
    ON fuel_transactions (site_id, occurred_at DESC);

CREATE TABLE outbox_messages (
    id uniqueidentifier NOT NULL CONSTRAINT pk_outbox_messages PRIMARY KEY,
    type nvarchar(120) NOT NULL,
    payload nvarchar(max) NOT NULL,
    idempotency_key nvarchar(200) NOT NULL,
    status nvarchar(32) NOT NULL,
    attempts int NOT NULL,
    occurred_at datetimeoffset NOT NULL,
    next_attempt_at datetimeoffset NOT NULL,
    processed_at datetimeoffset NULL,
    last_error nvarchar(2000) NULL,
    CONSTRAINT ux_outbox_messages_idempotency_key UNIQUE (idempotency_key),
    CONSTRAINT ck_outbox_messages_status CHECK (status IN ('Pending', 'Processing', 'Sent', 'DeadLetter'))
);

CREATE INDEX ix_outbox_messages_status_next_attempt
    ON outbox_messages (status, next_attempt_at)
    INCLUDE (idempotency_key, occurred_at, attempts);

CREATE TABLE reconciliation_runs (
    id uniqueidentifier NOT NULL CONSTRAINT pk_reconciliation_runs PRIMARY KEY,
    site_id uniqueidentifier NOT NULL,
    period_start datetimeoffset NOT NULL,
    period_end datetimeoffset NOT NULL,
    site_litres decimal(18, 3) NOT NULL,
    erp_litres decimal(18, 3) NOT NULL,
    variance_litres decimal(18, 3) NOT NULL,
    variance_percent decimal(9, 4) NOT NULL,
    threshold_percent decimal(9, 4) NOT NULL,
    status nvarchar(32) NOT NULL,
    created_at datetimeoffset NOT NULL,
    CONSTRAINT fk_reconciliation_runs_sites FOREIGN KEY (site_id) REFERENCES sites (id),
    CONSTRAINT ck_reconciliation_runs_period CHECK (period_start < period_end),
    CONSTRAINT ck_reconciliation_runs_litres CHECK (site_litres >= 0 AND erp_litres >= 0),
    CONSTRAINT ck_reconciliation_runs_threshold CHECK (threshold_percent >= 0 AND threshold_percent <= 100),
    CONSTRAINT ck_reconciliation_runs_status CHECK (status IN ('Balanced', 'VarianceDetected'))
);

CREATE INDEX ix_reconciliation_runs_site_period
    ON reconciliation_runs (site_id, period_end DESC);
