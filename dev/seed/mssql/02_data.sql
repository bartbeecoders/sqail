USE sqail_test;
GO
SET NOCOUNT ON;
WITH n AS (SELECT TOP (10000) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS g
           FROM sys.all_objects a CROSS JOIN sys.all_objects b)
SELECT g INTO #n FROM n;

INSERT INTO sales.customers (name, email, country)
SELECT CONCAT('Customer ', g), CONCAT('customer', g, '@example.test'),
       CHOOSE(1 + g % 5, 'BE', 'NL', 'FR', 'DE', 'US')
FROM #n WHERE g <= 1000;

INSERT INTO sales.products (sku, name, price, active)
SELECT CONCAT('SKU-', RIGHT(CONCAT('00000', g), 5)), CONCAT('Product ', g),
       ROUND((1 + g % 250) * 1.37, 2), CASE WHEN g % 17 <> 0 THEN 1 ELSE 0 END
FROM #n WHERE g <= 200;

INSERT INTO sales.orders (customer_id, ordered_at, status)
SELECT 1 + g % 1000, DATEADD(minute, -g, SYSDATETIMEOFFSET()),
       CHOOSE(1 + g % 4, 'new', 'paid', 'shipped', 'cancelled')
FROM #n;

INSERT INTO sales.order_items (order_id, product_id, quantity, unit_price)
SELECT DISTINCT o.id, p.id, 1 + k.k, p.price
FROM sales.orders o
CROSS JOIN (VALUES (1), (2), (3)) k(k)
JOIN sales.products p ON p.id = 1 + (o.id * k.k) % 200
WHERE NOT EXISTS (SELECT 1 FROM (VALUES (1), (2), (3)) k2(k)
                  WHERE k2.k < k.k AND 1 + (o.id * k2.k) % 200 = p.id);

INSERT INTO sales.type_zoo (c_bit, c_tinyint, c_smallint, c_bigint, c_float, c_real, c_decimal, c_money,
                            c_varchar, c_nvarchar, c_varbinary, c_date, c_time, c_datetime, c_datetime2,
                            c_dto, c_guid, c_xml)
VALUES
 (1, 255, 32767, 9223372036854775807, 3.14159, 2.5, 1234567890123456789012345678.0123456789, 922337203685477.5807,
  'hello', N'héllo ✓ 世界', 0xDEADBEEF, '2026-01-31', '23:59:59.1234567', '2026-01-31 12:00:00',
  '2026-01-31 12:00:00.1234567', '2026-01-31 12:00:00 +01:00', 'A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11',
  N'<root><a>1</a></root>'),
 (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
GO
