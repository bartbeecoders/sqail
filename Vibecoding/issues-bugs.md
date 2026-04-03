When connecting to the sql test database (alpha), the schema is not being loaded.
The database should be shown as tables, views, Procedures. ANd then grouped by schema.

I would first group by type (table, view, function, stored proc), and then by schema

Formatting should keep the sql valid. 
Example 
SELECT
    [id] AS 'Id',
    [plant_cd] AS 'Plant Cd'
FROM [mas].[equipment_group];

should be formatted as:
SELECT
    mg.[id]       AS 'Id',
    mg.[plant_cd] AS 'PlantCd'
FROM [mas].[equipment_group] mg;


Guid fields and datetime fields should be formatted as such. Now they show as DateTime(Some(DateTime { days: 45892, seconds_fragments: 10766758 })) or Guid(Some(6795f83c-8981-f011-9126-0050569afd62))


The sql code generated with the right mouse click on a table should be formatted properly and should fit the database type (SQL Server, PostgreSQL, MySQL, etc.)

Improve the editor tab design and functionality
- when clicking the + icon, a new tab should be created with the name Query x (where x is the next number)

- add the possibility to rename a tab by double clicking on it

Allow to set a LLM provider as the default for the application.
When starting the application, the default LLM provider should be used.

In the sql editor, the syntax highlighting should be based on the database type (SQL Server, PostgreSQL, MySQL, etc.)
It does not seem to detect syntax errors properly. I do not see any error highlighting.

Improve the autocomplete functionality in the sql editor.
when in a sql statement, propose the best matching next character or word. This could be a table name (after FROM or schema name), a column name (after SELECT or WHERE), a keyword (after FROM, WHERE, etc.), or a function name (after SELECT, WHERE, etc.).
Create a full autocomplete system that can be used in the sql editor.