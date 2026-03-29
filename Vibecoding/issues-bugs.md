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