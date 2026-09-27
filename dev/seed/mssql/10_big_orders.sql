-- sqail test data for SQL Server: sales.big_orders, 1,000,000 rows for testing
-- large results (streaming, grid scrolling, sorting, export). Idempotent:
-- scripts/db.sh also runs it on databases that were seeded before it existed.
-- The values are derived from the row number, so every run gives the same data.
USE sqail_test;
GO
IF OBJECT_ID('sales.big_orders') IS NULL
BEGIN
    CREATE TABLE sales.big_orders (
        id          int NOT NULL PRIMARY KEY,
        customer_id int NOT NULL,          -- 1..1000, like sales.customers
        product_id  int NOT NULL,          -- 1..200, like sales.products
        ordered_at  datetime2(0) NOT NULL,
        status      nvarchar(20) NOT NULL,
        quantity    int NOT NULL,
        unit_price  decimal(10,2) NOT NULL,
        total       decimal(12,2) NOT NULL,
        paid        bit NOT NULL,
        note        nvarchar(100) NULL     -- NULL in 9 of 10 rows
    );

    -- 10^6 row numbers from a cross join of digits; TABLOCK keeps the insert
    -- minimally logged, so this takes seconds.
    WITH d AS (SELECT n FROM (VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)) v(n)),
    g AS (
        SELECT 1 + a.n + 10 * b.n + 100 * c.n + 1000 * e.n + 10000 * f.n + 100000 * h.n AS i
        FROM d a CROSS JOIN d b CROSS JOIN d c CROSS JOIN d e CROSS JOIN d f CROSS JOIN d h
    ),
    r AS (
        SELECT i,
               1 + i % 1000 AS customer_id,
               1 + (i * 7) % 200 AS product_id,
               1 + (i * 13) % 20 AS quantity,
               CAST(1 + (i * 37) % 50000 AS decimal(10,2)) / 100 AS unit_price
        FROM g
    )
    INSERT INTO sales.big_orders WITH (TABLOCK)
        (id, customer_id, product_id, ordered_at, status, quantity, unit_price, total, paid, note)
    SELECT i, customer_id, product_id,
           DATEADD(minute, i * 3, CAST('2020-01-01T00:00:00' AS datetime2(0))),
           CHOOSE(1 + i % 5, N'new', N'paid', N'shipped', N'delivered', N'cancelled'),
           quantity, unit_price, quantity * unit_price,
           CAST(CASE WHEN i % 5 IN (1, 2, 3) THEN 1 ELSE 0 END AS bit),
           CASE WHEN i % 10 = 0 THEN CONCAT(N'Order ', i, N': please deliver after 17:00') END
    FROM r;

    CREATE INDEX ix_big_orders_customer ON sales.big_orders(customer_id);
    CREATE INDEX ix_big_orders_ordered_at ON sales.big_orders(ordered_at);
END
GO
