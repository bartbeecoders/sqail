-- sqail test schema for PostgreSQL. Mirrors dev/seed/mssql and dev/seed/sqlite.
CREATE SCHEMA sales;

CREATE TABLE sales.customers (
    id          serial PRIMARY KEY,
    name        text NOT NULL,
    email       text UNIQUE,
    country     char(2) NOT NULL DEFAULT 'BE',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sales.products (
    id          serial PRIMARY KEY,
    sku         varchar(32) NOT NULL UNIQUE,
    name        text NOT NULL,
    price       numeric(10,2) NOT NULL CHECK (price >= 0),
    active      boolean NOT NULL DEFAULT true
);

CREATE TABLE sales.orders (
    id          serial PRIMARY KEY,
    customer_id int NOT NULL REFERENCES sales.customers(id),
    ordered_at  timestamptz NOT NULL DEFAULT now(),
    status      text NOT NULL DEFAULT 'new'
);
CREATE INDEX ix_orders_customer ON sales.orders(customer_id);

CREATE TABLE sales.order_items (
    order_id    int NOT NULL REFERENCES sales.orders(id) ON DELETE CASCADE,
    product_id  int NOT NULL REFERENCES sales.products(id),
    quantity    int NOT NULL CHECK (quantity > 0),
    unit_price  numeric(10,2) NOT NULL,
    PRIMARY KEY (order_id, product_id)
);

-- One column per interesting type, for result-type mapping tests.
CREATE TABLE sales.type_zoo (
    id          serial PRIMARY KEY,
    c_bool      boolean,
    c_int2      smallint,
    c_int8      bigint,
    c_float     double precision,
    c_numeric   numeric(38,10),
    c_text      text,
    c_bytes     bytea,
    c_date      date,
    c_time      time,
    c_ts        timestamp,
    c_tstz      timestamptz,
    c_uuid      uuid,
    c_json      jsonb,
    c_array     int[],
    c_interval  interval
);

CREATE VIEW sales.order_totals AS
SELECT o.id AS order_id, o.customer_id, sum(i.quantity * i.unit_price) AS total
FROM sales.orders o JOIN sales.order_items i ON i.order_id = o.id
GROUP BY o.id, o.customer_id;

CREATE FUNCTION sales.customer_revenue(p_customer int) RETURNS numeric
LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(total), 0) FROM sales.order_totals WHERE customer_id = p_customer
$$;
