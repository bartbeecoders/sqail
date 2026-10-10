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

Make the object browser pane collapsable.
Make the AI assistant pane collapsable.
There is no horizontal scrolling on the resilts table


in the text editor, add text zooming. User the middle mouse button scroll to zoom in and out of the text



Improvments on the connections
- add the possibility to copy connections
- double clicking on a connection should open the edit dialog
- add auto discovery:
    - use the azure connection to query the databases present in a subscription (MS sql, postgres etc)

Closing a tab does not work.

Add a settings page where we can keep application wide settings:
- theme of the app (ligh, dark, other color themes, omarchy default)
- whedn closing a tab, the system ask for confirmation, this ask should be optional --> setting
- and other settings you can find

Connection tree improvements
- add a small toolbar to the treeview
    - add new fodler button
    - add new connection button
- allow in place rename of connections and folders
- allow drag and drop of connections tto other folders
