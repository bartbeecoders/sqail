-- sqail test schema for SQLite. Built into dev/data/sqail_test.db by scripts/db.sh.
PRAGMA foreign_keys = ON;

CREATE TABLE customers (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    email       TEXT UNIQUE,
    country     TEXT NOT NULL DEFAULT 'BE',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE products (
    id          INTEGER PRIMARY KEY,
    sku         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    price       REAL NOT NULL CHECK (price >= 0),
    active      INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE orders (
    id          INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    ordered_at  TEXT NOT NULL DEFAULT (datetime('now')),
    status      TEXT NOT NULL DEFAULT 'new'
);
CREATE INDEX ix_orders_customer ON orders(customer_id);
CREATE TABLE order_items (
    order_id    INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id  INTEGER NOT NULL REFERENCES products(id),
    quantity    INTEGER NOT NULL CHECK (quantity > 0),
    unit_price  REAL NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
CREATE TABLE type_zoo (
    id        INTEGER PRIMARY KEY,
    c_int     INTEGER,
    c_real    REAL,
    c_text    TEXT,
    c_blob    BLOB,
    c_numeric NUMERIC,
    c_bool    BOOLEAN,
    c_date    DATE,
    c_json    TEXT
);
CREATE VIEW order_totals AS
SELECT o.id AS order_id, o.customer_id, sum(i.quantity * i.unit_price) AS total
FROM orders o JOIN order_items i ON i.order_id = o.id
GROUP BY o.id, o.customer_id;


INSERT INTO customers (name, email, country)
WITH RECURSIVE n(g) AS (SELECT 1 UNION ALL SELECT g + 1 FROM n WHERE g < 1000)
SELECT 'Customer ' || g, 'customer' || g || '@example.test',
       CASE g % 5 WHEN 0 THEN 'BE' WHEN 1 THEN 'NL' WHEN 2 THEN 'FR' WHEN 3 THEN 'DE' ELSE 'US' END
FROM n;

INSERT INTO products (sku, name, price, active)
WITH RECURSIVE n(g) AS (SELECT 1 UNION ALL SELECT g + 1 FROM n WHERE g < 200)
SELECT 'SKU-' || substr('00000' || g, -5), 'Product ' || g, round((1 + g % 250) * 1.37, 2), g % 17 <> 0
FROM n;

INSERT INTO orders (customer_id, ordered_at, status)
WITH RECURSIVE n(g) AS (SELECT 1 UNION ALL SELECT g + 1 FROM n WHERE g < 10000)
SELECT 1 + g % 1000, datetime('now', '-' || g || ' minutes'),
       CASE g % 4 WHEN 0 THEN 'new' WHEN 1 THEN 'paid' WHEN 2 THEN 'shipped' ELSE 'cancelled' END
FROM n;

INSERT OR IGNORE INTO order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, 1 + (o.id * k.k) % 200, 1 + k.k, p.price
FROM orders o
CROSS JOIN (SELECT 1 AS k UNION ALL SELECT 2 UNION ALL SELECT 3) k
JOIN products p ON p.id = 1 + (o.id * k.k) % 200;

INSERT INTO type_zoo (c_int, c_real, c_text, c_blob, c_numeric, c_bool, c_date, c_json) VALUES
 (9223372036854775807, 3.14159, 'héllo ✓ 世界', X'DEADBEEF', 12345.678, 1, '2026-01-31', '{"a":[1,2,{"b":null}]}'),
 (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
