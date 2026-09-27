-- sqail test schema for SQL Server. Run by scripts/db.sh via sqlcmd.
IF DB_ID('sqail_test') IS NULL CREATE DATABASE sqail_test;
GO
USE sqail_test;
GO
IF SUSER_ID('sqail') IS NULL
    CREATE LOGIN sqail WITH PASSWORD = 'Sqail2_dev!Passw0rd', CHECK_POLICY = OFF;
IF USER_ID('sqail') IS NULL
BEGIN
    CREATE USER sqail FOR LOGIN sqail;
    ALTER ROLE db_owner ADD MEMBER sqail;
END
GO
CREATE SCHEMA sales;
GO
CREATE TABLE sales.customers (
    id          int IDENTITY PRIMARY KEY,
    name        nvarchar(200) NOT NULL,
    email       nvarchar(320) UNIQUE,
    country     char(2) NOT NULL DEFAULT 'BE',
    created_at  datetimeoffset NOT NULL DEFAULT SYSDATETIMEOFFSET()
);
CREATE TABLE sales.products (
    id          int IDENTITY PRIMARY KEY,
    sku         varchar(32) NOT NULL UNIQUE,
    name        nvarchar(200) NOT NULL,
    price       decimal(10,2) NOT NULL CHECK (price >= 0),
    active      bit NOT NULL DEFAULT 1
);
CREATE TABLE sales.orders (
    id          int IDENTITY PRIMARY KEY,
    customer_id int NOT NULL REFERENCES sales.customers(id),
    ordered_at  datetimeoffset NOT NULL DEFAULT SYSDATETIMEOFFSET(),
    status      nvarchar(20) NOT NULL DEFAULT 'new'
);
CREATE INDEX ix_orders_customer ON sales.orders(customer_id);
CREATE TABLE sales.order_items (
    order_id    int NOT NULL REFERENCES sales.orders(id) ON DELETE CASCADE,
    product_id  int NOT NULL REFERENCES sales.products(id),
    quantity    int NOT NULL CHECK (quantity > 0),
    unit_price  decimal(10,2) NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
CREATE TABLE sales.type_zoo (
    id              int IDENTITY PRIMARY KEY,
    c_bit           bit,
    c_tinyint       tinyint,
    c_smallint      smallint,
    c_bigint        bigint,
    c_float         float,
    c_real          real,
    c_decimal       decimal(38,10),
    c_money         money,
    c_varchar       varchar(100),
    c_nvarchar      nvarchar(max),
    c_varbinary     varbinary(max),
    c_date          date,
    c_time          time(7),
    c_datetime      datetime,
    c_datetime2     datetime2(7),
    c_dto           datetimeoffset,
    c_guid          uniqueidentifier,
    c_xml           xml
);
GO
CREATE VIEW sales.order_totals AS
SELECT o.id AS order_id, o.customer_id, SUM(i.quantity * i.unit_price) AS total
FROM sales.orders o JOIN sales.order_items i ON i.order_id = o.id
GROUP BY o.id, o.customer_id;
GO
CREATE PROCEDURE sales.customer_orders @customer_id int AS
BEGIN
    SET NOCOUNT ON;
    SELECT * FROM sales.orders WHERE customer_id = @customer_id ORDER BY ordered_at DESC;
    SELECT * FROM sales.order_totals WHERE customer_id = @customer_id;
END
GO
