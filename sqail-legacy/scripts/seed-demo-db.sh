#!/usr/bin/env bash
# seed-demo-db.sh — generate marketing/demo-data/sqail-demo.sqlite, a small
# bookstore database used for marketing screenshots and the animated demo GIF.
#
# The schema is deliberately obvious so AI screenshots read well:
#   authors   → books (many)
#   customers → orders → books (join target)
#   books     → reviews ← customers
#
# Safe to run repeatedly — overwrites the demo file, touches nothing else.
#
# Usage:
#   ./scripts/seed-demo-db.sh [output.sqlite]
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEMO_DIR="$PROJECT_ROOT/marketing/demo-data"
OUTPUT="${1:-$DEMO_DIR/sqail-demo.sqlite}"

command -v sqlite3 >/dev/null 2>&1 || {
  echo "Error: sqlite3 not found on PATH." >&2
  exit 1
}

mkdir -p "$(dirname "$OUTPUT")"
rm -f "$OUTPUT"

sqlite3 "$OUTPUT" <<'SQL'
PRAGMA foreign_keys = ON;

-- ─── Schema ────────────────────────────────────────────────────────────
CREATE TABLE authors (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  country     TEXT NOT NULL,
  birth_year  INTEGER NOT NULL
);

CREATE TABLE books (
  id              INTEGER PRIMARY KEY,
  title           TEXT NOT NULL,
  author_id       INTEGER NOT NULL REFERENCES authors(id),
  genre           TEXT NOT NULL,
  published_year  INTEGER NOT NULL,
  price           REAL NOT NULL,
  stock           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE customers (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  email      TEXT NOT NULL UNIQUE,
  city       TEXT NOT NULL,
  joined_at  TEXT NOT NULL
);

CREATE TABLE orders (
  id           INTEGER PRIMARY KEY,
  customer_id  INTEGER NOT NULL REFERENCES customers(id),
  book_id      INTEGER NOT NULL REFERENCES books(id),
  quantity     INTEGER NOT NULL,
  order_date   TEXT NOT NULL,
  total_price  REAL NOT NULL
);

CREATE TABLE reviews (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id),
  customer_id  INTEGER NOT NULL REFERENCES customers(id),
  rating       INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
  body         TEXT NOT NULL,
  created_at   TEXT NOT NULL
);

CREATE INDEX idx_books_author ON books(author_id);
CREATE INDEX idx_orders_customer ON orders(customer_id);
CREATE INDEX idx_orders_book ON orders(book_id);
CREATE INDEX idx_reviews_book ON reviews(book_id);

-- ─── Authors ───────────────────────────────────────────────────────────
INSERT INTO authors (id, name, country, birth_year) VALUES
  (1,  'Ursula K. Le Guin',   'USA',         1929),
  (2,  'Haruki Murakami',     'Japan',       1949),
  (3,  'Chimamanda Adichie',  'Nigeria',     1977),
  (4,  'Jorge Luis Borges',   'Argentina',   1899),
  (5,  'Italo Calvino',       'Italy',       1923),
  (6,  'Octavia Butler',      'USA',         1947),
  (7,  'Kazuo Ishiguro',      'UK',          1954),
  (8,  'Ted Chiang',          'USA',         1967),
  (9,  'Liu Cixin',           'China',       1963),
  (10, 'Zadie Smith',         'UK',          1975),
  (11, 'Cormac McCarthy',     'USA',         1933),
  (12, 'Margaret Atwood',     'Canada',      1939),
  (13, 'China Miéville',      'UK',          1972),
  (14, 'N. K. Jemisin',       'USA',         1972),
  (15, 'Yoko Ogawa',          'Japan',       1962);

-- ─── Books ─────────────────────────────────────────────────────────────
INSERT INTO books (id, title, author_id, genre, published_year, price, stock) VALUES
  (1,  'The Left Hand of Darkness',     1,  'Science Fiction', 1969, 14.99, 12),
  (2,  'The Dispossessed',               1,  'Science Fiction', 1974, 15.99, 8),
  (3,  'A Wizard of Earthsea',           1,  'Fantasy',         1968, 12.99, 20),
  (4,  'Kafka on the Shore',             2,  'Magical Realism', 2002, 16.99, 15),
  (5,  '1Q84',                           2,  'Literary Fiction',2009, 19.99, 6),
  (6,  'Norwegian Wood',                 2,  'Literary Fiction',1987, 14.99, 18),
  (7,  'Americanah',                     3,  'Literary Fiction',2013, 17.99, 22),
  (8,  'Half of a Yellow Sun',           3,  'Historical',      2006, 16.99, 11),
  (9,  'Ficciones',                      4,  'Short Stories',   1944, 13.99, 9),
  (10, 'The Aleph',                      4,  'Short Stories',   1949, 13.99, 7),
  (11, 'Invisible Cities',               5,  'Literary Fiction',1972, 14.99, 14),
  (12, 'If on a winter''s night',        5,  'Literary Fiction',1979, 15.99, 10),
  (13, 'Kindred',                        6,  'Science Fiction', 1979, 14.99, 16),
  (14, 'Parable of the Sower',           6,  'Science Fiction', 1993, 15.99, 13),
  (15, 'The Remains of the Day',         7,  'Literary Fiction',1989, 15.99, 19),
  (16, 'Klara and the Sun',              7,  'Science Fiction', 2021, 18.99, 25),
  (17, 'Never Let Me Go',                7,  'Science Fiction', 2005, 14.99, 17),
  (18, 'Stories of Your Life',           8,  'Science Fiction', 2002, 16.99, 21),
  (19, 'Exhalation',                     8,  'Science Fiction', 2019, 17.99, 14),
  (20, 'The Three-Body Problem',         9,  'Science Fiction', 2008, 16.99, 30),
  (21, 'The Dark Forest',                9,  'Science Fiction', 2008, 17.99, 24),
  (22, 'White Teeth',                    10, 'Literary Fiction',2000, 15.99, 11),
  (23, 'On Beauty',                      10, 'Literary Fiction',2005, 15.99, 9),
  (24, 'The Road',                       11, 'Post-Apocalyptic',2006, 14.99, 28),
  (25, 'Blood Meridian',                 11, 'Western',         1985, 16.99, 12),
  (26, 'The Handmaid''s Tale',           12, 'Dystopian',       1985, 15.99, 33),
  (27, 'Oryx and Crake',                 12, 'Science Fiction', 2003, 15.99, 15),
  (28, 'Perdido Street Station',         13, 'Fantasy',         2000, 17.99, 18),
  (29, 'The Fifth Season',               14, 'Fantasy',         2015, 16.99, 26),
  (30, 'The Memory Police',              15, 'Literary Fiction',1994, 15.99, 10);

-- ─── Customers ─────────────────────────────────────────────────────────
INSERT INTO customers (id, name, email, city, joined_at) VALUES
  (1,  'Alex Rivera',      'alex.rivera@example.com',      'Portland',    '2024-01-14'),
  (2,  'Priya Shah',       'priya.shah@example.com',       'Austin',      '2024-02-03'),
  (3,  'Marcus Chen',      'marcus.chen@example.com',      'Seattle',     '2024-02-22'),
  (4,  'Emilia Novak',     'emilia.novak@example.com',     'Boston',      '2024-03-11'),
  (5,  'Dmitri Kovalev',   'dmitri.k@example.com',         'Chicago',     '2024-03-29'),
  (6,  'Sophie Laurent',   'sophie.laurent@example.com',   'Montreal',    '2024-04-06'),
  (7,  'Hiroshi Tanaka',   'hiroshi.t@example.com',        'Vancouver',   '2024-04-18'),
  (8,  'Nadia Okafor',     'nadia.okafor@example.com',     'Toronto',     '2024-05-02'),
  (9,  'Tomás Vega',       'tomas.vega@example.com',       'Denver',      '2024-05-15'),
  (10, 'Isabelle Moreau',  'isabelle.m@example.com',       'Minneapolis', '2024-06-01'),
  (11, 'Samir El-Amin',    'samir.elamin@example.com',     'Philadelphia','2024-06-20'),
  (12, 'Rachel Goldberg',  'rachel.g@example.com',         'New York',    '2024-07-04'),
  (13, 'Yuki Watanabe',    'yuki.w@example.com',           'San Diego',   '2024-07-19'),
  (14, 'Oluwa Adeyemi',    'oluwa.a@example.com',          'Atlanta',     '2024-08-03'),
  (15, 'Claire Dufresne',  'claire.d@example.com',         'Ottawa',      '2024-08-22'),
  (16, 'Viktor Rasmussen', 'viktor.r@example.com',         'Milwaukee',   '2024-09-10'),
  (17, 'Leila Hosseini',   'leila.h@example.com',          'Houston',     '2024-09-28'),
  (18, 'Jonah Blackwood',  'jonah.b@example.com',          'Nashville',   '2024-10-12'),
  (19, 'Anika Dasgupta',   'anika.d@example.com',          'Madison',     '2024-10-30'),
  (20, 'Caleb Ortiz',      'caleb.o@example.com',          'Oakland',     '2024-11-18');

-- ─── Orders ────────────────────────────────────────────────────────────
INSERT INTO orders (id, customer_id, book_id, quantity, order_date, total_price) VALUES
  (1,  1,  1,  1, '2024-02-05', 14.99),
  (2,  1,  20, 2, '2024-02-18', 33.98),
  (3,  2,  7,  1, '2024-03-01', 17.99),
  (4,  2,  26, 1, '2024-03-15', 15.99),
  (5,  3,  4,  3, '2024-03-22', 50.97),
  (6,  3,  16, 1, '2024-04-04', 18.99),
  (7,  4,  29, 2, '2024-04-10', 33.98),
  (8,  4,  24, 1, '2024-04-25', 14.99),
  (9,  5,  11, 1, '2024-05-02', 14.99),
  (10, 5,  5,  1, '2024-05-14', 19.99),
  (11, 6,  19, 2, '2024-05-27', 35.98),
  (12, 6,  7,  1, '2024-06-08', 17.99),
  (13, 7,  20, 1, '2024-06-12', 16.99),
  (14, 7,  21, 1, '2024-06-19', 17.99),
  (15, 8,  14, 1, '2024-06-30', 15.99),
  (16, 8,  13, 1, '2024-07-07', 14.99),
  (17, 9,  1,  1, '2024-07-15', 14.99),
  (18, 9,  16, 2, '2024-07-22', 37.98),
  (19, 10, 26, 1, '2024-07-30', 15.99),
  (20, 10, 27, 1, '2024-08-11', 15.99),
  (21, 11, 29, 1, '2024-08-19', 16.99),
  (22, 11, 30, 1, '2024-09-01', 15.99),
  (23, 12, 4,  2, '2024-09-06', 33.98),
  (24, 12, 5,  1, '2024-09-17', 19.99),
  (25, 13, 18, 1, '2024-09-25', 16.99),
  (26, 13, 19, 1, '2024-10-02', 17.99),
  (27, 14, 7,  1, '2024-10-09', 17.99),
  (28, 14, 8,  1, '2024-10-18', 16.99),
  (29, 15, 24, 2, '2024-10-25', 29.98),
  (30, 15, 25, 1, '2024-11-02', 16.99),
  (31, 16, 29, 1, '2024-11-10', 16.99),
  (32, 16, 14, 1, '2024-11-16', 15.99),
  (33, 17, 20, 1, '2024-11-22', 16.99),
  (34, 17, 9,  2, '2024-12-01', 27.98),
  (35, 18, 25, 1, '2024-12-09', 16.99),
  (36, 18, 22, 1, '2024-12-15', 15.99),
  (37, 19, 29, 1, '2024-12-22', 16.99),
  (38, 19, 16, 1, '2025-01-04', 18.99),
  (39, 20, 20, 3, '2025-01-11', 50.97),
  (40, 20, 21, 1, '2025-01-19', 17.99);

-- ─── Reviews ───────────────────────────────────────────────────────────
INSERT INTO reviews (id, book_id, customer_id, rating, body, created_at) VALUES
  (1,  1,  1,  5, 'A quiet masterpiece. The ansible still haunts me.',        '2024-02-20'),
  (2,  20, 1,  5, 'Hard sci-fi at its sharpest. Bring a notebook.',           '2024-03-01'),
  (3,  7,  2,  4, 'Honest about identity in a way most novels aren''t.',     '2024-03-12'),
  (4,  26, 2,  5, 'More urgent every year it exists.',                       '2024-03-28'),
  (5,  4,  3,  5, 'Read it in two sittings. Loved every page.',              '2024-04-02'),
  (6,  16, 3,  4, 'Heartbreaking in the quiet Ishiguro way.',                '2024-04-14'),
  (7,  29, 4,  5, 'Finally a fantasy world I can''t predict.',               '2024-04-22'),
  (8,  24, 4,  4, 'Bleak, beautiful, impossible to put down.',               '2024-05-08'),
  (9,  11, 5,  5, 'A perfect little book. Tiny, dense, enormous.',           '2024-05-20'),
  (10, 5,  5,  3, 'Long. Sometimes too long. Worth it.',                     '2024-06-01'),
  (11, 19, 6,  5, 'Every story lands. Chiang is a national treasure.',      '2024-06-14'),
  (12, 7,  6,  5, 'Adichie never wastes a sentence.',                        '2024-06-22'),
  (13, 20, 7,  4, 'Bigger than the reviews warned me.',                      '2024-06-28'),
  (14, 21, 7,  4, 'Second one is where it gets wild.',                       '2024-07-05'),
  (15, 14, 8,  5, 'Butler predicted everything.',                            '2024-07-12'),
  (16, 13, 8,  5, 'Still the best time-travel novel I''ve read.',           '2024-07-18'),
  (17, 1,  9,  4, 'Took me a while to settle in, then couldn''t stop.',     '2024-07-25'),
  (18, 16, 9,  5, 'Klara broke my heart politely.',                          '2024-08-02'),
  (19, 26, 10, 5, 'Required reading, full stop.',                            '2024-08-14'),
  (20, 27, 10, 4, 'Atwood''s range is silly at this point.',                '2024-08-25'),
  (21, 29, 11, 5, 'I cried. Multiple times. At worldbuilding.',             '2024-09-03'),
  (22, 30, 11, 4, 'Dreamlike. Eerie. Stuck with me.',                        '2024-09-10'),
  (23, 4,  12, 5, 'Murakami at his weirdest and best.',                      '2024-09-20'),
  (24, 18, 13, 5, '"Story of Your Life" is the best novella ever written.', '2024-09-30'),
  (25, 19, 13, 4, 'Almost lives up to Stories of Your Life. Almost.',       '2024-10-10');

-- ─── Summary view ─────────────────────────────────────────────────────
CREATE VIEW top_books AS
SELECT
  b.id,
  b.title,
  a.name            AS author,
  SUM(o.quantity)   AS copies_sold,
  SUM(o.total_price) AS revenue
FROM books b
JOIN authors a ON a.id = b.author_id
LEFT JOIN orders o ON o.book_id = b.id
GROUP BY b.id
ORDER BY revenue DESC NULLS LAST;
SQL

# Verify
ROW_COUNT=$(sqlite3 "$OUTPUT" "SELECT (SELECT COUNT(*) FROM authors) || ' authors, ' || (SELECT COUNT(*) FROM books) || ' books, ' || (SELECT COUNT(*) FROM customers) || ' customers, ' || (SELECT COUNT(*) FROM orders) || ' orders, ' || (SELECT COUNT(*) FROM reviews) || ' reviews';")
SIZE=$(du -h "$OUTPUT" | cut -f1)

echo "Wrote $OUTPUT ($SIZE)"
echo "Contents: $ROW_COUNT"
