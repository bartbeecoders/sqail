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