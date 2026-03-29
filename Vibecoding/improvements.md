Add MS sqlserver as a database to connect to.
For dev purposes set the following connection details:
- Host: 192.168.0.33
- Port: 1433
- Database: alpha
- Username: report
- Password: report


[ ] Format document feature

[ ] Multi select feature

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

[ ] Multi platform deployment
Create build scripts for each platform: 
Try to create  single executable files for each platform, with instructions on how to run them.
Add a README file with instructions on how to build and run the application.
- [ ] Windows
- [ ] macOS
- [ ] Linux

[ ] App Versioning
Set the main version to 0.1.0. ANd set a build nr (yyyymmdd-revision)
Increase/set the build nr every time a new build is created.
Show the app version in the title bar
