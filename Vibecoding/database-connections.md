Add the possibility to use integrated security for SQL Server connections.
Add the possibility to trust the server certificate for SQL Server connections.

Add the ability to sign in with Microsoft Entra ID on a MS SQL Server database (azure).


Implement the windows/Integrated  authentication for SQL Server connections.

Windows/Integrated authentication is not supported in this build. Enable the 'integrated-auth-gssapi' feature for tiberius.

Allow to set the connections through connection strings:

postgresql://testuser:testpass123@192.168.0.34:5432/testdb




When connecting to a postgres db, I get:
error occurred while decoding column schema_name: error in Any driver mapping: Any driver does not support the Postgres type PgTypeInfo(Name)

Review the appliction and implement full support for postgres databases.

In the database connections dialog, on a postgres connection, show a dropdown of the existing databases on the server. (when host and port and user are set)