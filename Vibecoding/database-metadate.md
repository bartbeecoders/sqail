Add the following feature:

Add a Metadata generation button that generates for the whole database (tables, views, stored procedures, functions, etc.) a complete documentation per object.

- Overall description of the object
- description of each column/parameter
- example usage
- related objects (tables, views, stored procedures, functions, etc.)
- dependencies (tables, views, stored procedures, functions, etc.)

This metadate should be automatically generated when the user clicks the "Generate Metadata" button, by giving the object structure to an AI model (like OpenAI) and asking it to generate the documentation.

Store the generated metadata in the surrealdb embedded database.
- object_name
- object_type
- metadata (the generated metadata)
- generated_at (timestamp)

Allow for the user to review and update the metadata for each object.

The purpose of this feature is to provide a complete documentation for the whole database, so that the user can understand the database structure and usage. This data can then be used by the AI assistant to provide better suggestions and recommendations.

Add the ability to generate the metadata on 1 table, by right-clicking on the table name in the database explorer and selecting "Generate Metadata" from the context menu.

Make the metadata dialog box more spacious and readable. Use tabs for different sections of the metadata.

Add a generate metadata button next to the schema or the table name in the database explorer. This would then generate the metadata for that specific object. (or for the whole schema)
Indicate with a spinner next to the object that the data is being generated.

When metadata exists for an object, show a green sparkles icon next to the object name.
When no metadata exists for an object, show a gray sparkles icon next to the object name. 
Clicking on the sparkles icon generates or re-generated the metadata.
Clicking on the sparkle icon next to a schema, should start generating the metadata for all tables in that schema.


