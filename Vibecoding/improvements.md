Depending on where you drag the table, view or stored proc, adjust the action.

For instance when dragging a table after a from or in a select statement, only add the schema.name
Dragging is in a new line, emty spot, create a complete formatted select statement for that table

Same for views

For stored procs, turn it into a exec sp_name

Project redesign
We created yesterday the sqail2 project which runs fine now.
I want to clean up this sqail project. 
- Move the old sqail code to a sqail-legacy folder
- Move the new sqail2 code as the base project (in Sqail)
- Update the documentation
- Update the sqail-portal to refect the new version
- help me update the github deployement


Add the ability to connect to a azure sql database 