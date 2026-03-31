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