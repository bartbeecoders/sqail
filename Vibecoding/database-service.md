Create a separate project in dotnet 10 (c#) that can be used as a database service for sqail.

It should be able to do the following:
- Connect to a database (Start with MS SQL Server first, later we will add other databases)
- Execute queries
- Return results
- Handle errors
- Return the sql objects structure
- Allow to run queries, select, update, insert, delete
- ALlow to run stored procedures and functions
- expose all this through a REST API
- the service should be able to handle multiple connections at the same time
- all configuration should be stored in a json file
- the service should be able to store also the metadata of the database in a json file

Put this in a folder called "sqail-dbservice"
Use fastendpoint for the REST API  (https://fast-endpoints.com/)


Update the dbservice:
- [ ] Add a connections.json file to store the connections
- [ ] Add a metadata.json file to store the metadata of the database    
- [ ] Add a POST,PUT,DELETE endpoints to manage the connections