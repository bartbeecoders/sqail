INSERT INTO sales.customers (name, email, country)
SELECT 'Customer ' || g, 'customer' || g || '@example.test',
       (ARRAY['BE','NL','FR','DE','US'])[1 + g % 5]
FROM generate_series(1, 1000) g;

INSERT INTO sales.products (sku, name, price, active)
SELECT 'SKU-' || lpad(g::text, 5, '0'), 'Product ' || g, round((1 + g % 250) * 1.37, 2), g % 17 <> 0
FROM generate_series(1, 200) g;

INSERT INTO sales.orders (customer_id, ordered_at, status)
SELECT 1 + g % 1000, now() - (g || ' minutes')::interval,
       (ARRAY['new','paid','shipped','cancelled'])[1 + g % 4]
FROM generate_series(1, 10000) g;

INSERT INTO sales.order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, 1 + (o.id * k) % 200, 1 + k, p.price
FROM sales.orders o
CROSS JOIN generate_series(1, 3) k
JOIN sales.products p ON p.id = 1 + (o.id * k) % 200
ON CONFLICT DO NOTHING;

INSERT INTO sales.type_zoo (c_bool, c_int2, c_int8, c_float, c_numeric, c_text, c_bytes,
                            c_date, c_time, c_ts, c_tstz, c_uuid, c_json, c_array, c_interval)
VALUES
 (true, 32767, 9223372036854775807, 3.14159, 12345678901234567890.0123456789, 'héllo ✓ 世界',
  '\xdeadbeef', '2026-01-31', '23:59:59', '2026-01-31 12:00:00', '2026-01-31 12:00:00+01',
  'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '{"a": [1, 2, {"b": null}]}', '{1,2,3}', '1 day 02:03:04'),
 (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
