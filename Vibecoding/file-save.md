## File management

Add the ability to save and load files.

Use standard text files .sql as extension for SQL scripts.
Or use a custom format like .sqail


SQL scripts:
- when saving as a text file, save the SQL script as is
- when saving as a .sqail file, save the SQL script in a custom format that can be loaded back, include the connection information

Diagram:
- when saving as a .sqail file, save the diagram in a custom format that can be loaded back, include the connection information

.sqail files can also include the AI prompting history for that file. This means that prompting histories need to be kept by file.



## Project management

Add a concept of projects to the application. A project is a collection of files that are related to each other.
Projects can also be saved as sqail files.


## Sqail File Format
Use human readable json structure for sqail files.
Encode binary data as base64 strings.
Encrypt connection strings and other sensitive data.
