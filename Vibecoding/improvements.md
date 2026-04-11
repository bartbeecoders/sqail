Add MS sqlserver as a database to connect to.
For dev purposes set the following connection details:
- Host: 192.168.0.33
- Port: 1433
- Database: alpha
- Username: report
- Password: report


[x] Format document feature
Implement the document format feature (Format tool in the menu or Ctrl+Shift+F)
Formatting a select statement should align the columns and keywords in a readable way.
Example:
SELECT
    eg.[id]                        AS 'Id',
    eg.[plant_cd]                  AS 'PlantCd',
    eg.[equipment_class_cd]        AS 'EquipmentClassCd',
    eg.[equipment_group_cd]        AS 'EquipmentGroupCd',
    eg.[description]               AS 'Description',
    eg.[application_cd]            AS 'ApplicationCd',
    eg.[group_type]                AS 'GroupType',
    eg.[is_active]                 AS 'IsActive',
    eg.[created_date]              AS 'CreatedDate',
    eg.[last_edit_date]            AS 'LastEditDate',
    eg.[last_edit_user_cd]         AS 'LastEditUserCd'
FROM [mas].[equipment_group] eg;

Do not include spaces in the alias names. And add alieses for the table names.


[x] Multi select feature

[x] Implement the ability to export results to excel, csv, json, xml

[x]  ability to drag tables or views from the schema tree to the query editor. This should create a SELECT statement with the table or view name, and all columns from that table or view.
Example layout:
SELECT 
    column1 as 'Column1',
    column2 as 'Column2' 
FROM [schema].[table]

The SQL syntax should match the selected database type.


[x] The results table should have a header row with the column names, and then the data rows below that.
The columns should be well aligned and easy to read.
Add Sorting and Filtering capabilities to the results table.
Add a search bar to filter the results table.

[x] Multi platform deployment
Create build scripts for each platform: 
Try to create  single executable files for each platform, with instructions on how to run them.
Add a README file with instructions on how to build and run the application.
- [x] Windows
- [x] macOS
- [x] Linux

[x] App Versioning
Set the main version to 0.1.0. ANd set a build nr (yyyymmdd-revision)
Increase/set the build nr every time a new build is created.
Show the app version in the title bar


[x] Add more AI providers:
Minimax: 
api: https://api.minimax.io/v1
docs: https://platform.minimax.io/docs/api-reference/text-openai-api
models: https://platform.minimax.io/docs/guides/models-intro

Z.ai:
api: https://api.z.ai/api/paas/v4/
docs: https://docs.z.ai/api-reference/introduction
models: https://docs.z.ai/api-reference/introduction

claude code cli:
use the claude code cli as an AI llm provider

LM STudio:
use LM Studio as an AI llm provider
api: https://llm.hideterms.com

Make sure you add a test button for each AI provider to test the connection and configuration.

[x] Add keyboard shortcuts for common actions:
- Ctrl + N: New connection
- Ctrl + S: Save query
- Ctrl + O: Open query
- F5: Run query
- Ctrl + Shift + S: Save query as
- Ctrl + Shift + F: Format query

Add a setting page to configure the application.
Add a tab for the keyboard shortcuts configuration.

[x] Save the applicactions state when closing the app, and restore it when opening the app. (maximazed, window size, etc.)

[x] Use mouse wheel scroll + shift to zoom in and out the query editor (larger/smaller font size)

[x] On the editor tabs, implement a context menu with Close all tabs, Close other tabs, etc.

[x] Allow to drag sql object to the tab level, this should open a new tab with the sql object as the content.

### results table improvements
Clicking on a cell in the results table should select the whole row.
Allow for the selection of multiple rows.
Add a context menu to the results table with options to copy.

### saved queries
Loading a saved query should open a new tab with the query content. If the query is already open, then switch to that tab.
When saving a query, if a query with the same name already exists, ask the user if they want to overwrite it.
When saving a query, replace the tab name with the new query name, unless the tab has already a updated name, then use that name as the saved query name.
The save query should also remember the connection that was used for the query.

### multi connections issues.
A query in a tab is always linked to a connection. 
Multiple open tabs can be linked to different connections.

It should be possible to connect to multiple connections at the same time.
When switching from one query to another query that is linked to a different connection, the connection should be switched to the new connection. And the object tree should show the objects of that connection.

Make the object list pane width draggable

Would it technically make sense to integrate page-agent in the app (https://github.com/alibaba/page-agent)?
So you can control, UI test, and automate the app using AI.


Check the split editor view. Splitting the editor view should show the same sql in both editors, so that the user can see different parts of the same query.

Double clicking the app title should maximize or restore the window.

When in normal mode (not maximized), it should be posstible to resize  the window by dragging the sides of the window.

When there are too many tabs, overflow the tabs to a 2nd line. So have the ability have multiple rows.

Add the ability to pin tabs, pinned tabs should be on a top line.

When I run a query, and a sql error is shown, it could be usefull that the user can pass this error to an AI assistant to help fix the query.