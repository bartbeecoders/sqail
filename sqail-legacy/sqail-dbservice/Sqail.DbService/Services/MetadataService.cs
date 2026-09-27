using System.Text.Json;
using Dapper;
using Sqail.DbService.Configuration;
using Sqail.DbService.Models;

namespace Sqail.DbService.Services;

public interface IMetadataService
{
    Task<DatabaseMetadata> GetMetadataAsync(string connectionId, bool forceRefresh = false);
    Task SaveMetadataCacheAsync(string connectionId, DatabaseMetadata metadata);
    Task<DatabaseMetadata?> LoadMetadataCacheAsync(string connectionId);
}

public class MetadataService(IConnectionManager connectionManager, AppConfig config) : IMetadataService
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    public async Task<DatabaseMetadata> GetMetadataAsync(string connectionId, bool forceRefresh = false)
    {
        if (!forceRefresh)
        {
            var cached = await LoadMetadataCacheAsync(connectionId);
            if (cached is not null)
                return cached;
        }

        var connection = connectionManager.GetConnection(connectionId);
        var dbName = await connection.ExecuteScalarAsync<string>("SELECT DB_NAME()") ?? "unknown";
        var serverVersion = await connection.ExecuteScalarAsync<string>("SELECT @@VERSION");

        var schemas = await GetSchemasAsync(connectionId);

        var metadata = new DatabaseMetadata
        {
            ConnectionId = connectionId,
            DatabaseName = dbName,
            ServerVersion = serverVersion,
            RetrievedAt = DateTime.UtcNow,
            Schemas = schemas
        };

        await SaveMetadataCacheAsync(connectionId, metadata);
        return metadata;
    }

    private async Task<List<SchemaInfo>> GetSchemasAsync(string connectionId)
    {
        var connection = connectionManager.GetConnection(connectionId);

        var schemaNames = (await connection.QueryAsync<string>(
            "SELECT DISTINCT TABLE_SCHEMA FROM INFORMATION_SCHEMA.TABLES ORDER BY TABLE_SCHEMA")).ToList();

        var schemas = new List<SchemaInfo>();
        foreach (var schemaName in schemaNames)
        {
            var tables = await GetTablesAsync(connection, schemaName);
            var views = await GetViewsAsync(connection, schemaName);
            var procedures = await GetProceduresAsync(connection, schemaName);
            var functions = await GetFunctionsAsync(connection, schemaName);

            schemas.Add(new SchemaInfo
            {
                Name = schemaName,
                Tables = tables,
                Views = views,
                StoredProcedures = procedures,
                Functions = functions
            });
        }

        return schemas;
    }

    private static async Task<List<TableInfo>> GetTablesAsync(System.Data.IDbConnection connection, string schema)
    {
        var tableNames = await connection.QueryAsync<string>(
            "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = @Schema AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
            new { Schema = schema });

        var tables = new List<TableInfo>();
        foreach (var tableName in tableNames)
        {
            var columns = await GetTableColumnsAsync(connection, schema, tableName);
            var indexes = await GetIndexesAsync(connection, schema, tableName);
            var foreignKeys = await GetForeignKeysAsync(connection, schema, tableName);

            tables.Add(new TableInfo
            {
                Schema = schema,
                Name = tableName,
                Columns = columns,
                Indexes = indexes,
                ForeignKeys = foreignKeys
            });
        }

        return tables;
    }

    private static async Task<List<TableColumnInfo>> GetTableColumnsAsync(
        System.Data.IDbConnection connection, string schema, string tableName)
    {
        const string sql = """
            SELECT
                c.COLUMN_NAME AS Name,
                c.DATA_TYPE AS DataType,
                c.CHARACTER_MAXIMUM_LENGTH AS MaxLength,
                c.NUMERIC_PRECISION AS [Precision],
                c.NUMERIC_SCALE AS Scale,
                CASE WHEN c.IS_NULLABLE = 'YES' THEN 1 ELSE 0 END AS IsNullable,
                CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END AS IsPrimaryKey,
                COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'IsIdentity') AS IsIdentity,
                c.COLUMN_DEFAULT AS DefaultValue
            FROM INFORMATION_SCHEMA.COLUMNS c
            LEFT JOIN (
                SELECT ku.TABLE_SCHEMA, ku.TABLE_NAME, ku.COLUMN_NAME
                FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku
                    ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME
                WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
            ) pk ON c.TABLE_SCHEMA = pk.TABLE_SCHEMA AND c.TABLE_NAME = pk.TABLE_NAME AND c.COLUMN_NAME = pk.COLUMN_NAME
            WHERE c.TABLE_SCHEMA = @Schema AND c.TABLE_NAME = @TableName
            ORDER BY c.ORDINAL_POSITION
            """;

        var rows = await connection.QueryAsync(sql, new { Schema = schema, TableName = tableName });
        return rows.Select(r => new TableColumnInfo
        {
            Name = r.Name,
            DataType = r.DataType,
            MaxLength = (int?)r.MaxLength,
            Precision = (int?)(byte?)r.Precision,
            Scale = (int?)r.Scale,
            IsNullable = r.IsNullable == 1,
            IsPrimaryKey = r.IsPrimaryKey == 1,
            IsIdentity = r.IsIdentity == 1,
            DefaultValue = (string?)r.DefaultValue
        }).ToList();
    }

    private static async Task<List<IndexInfo>> GetIndexesAsync(
        System.Data.IDbConnection connection, string schema, string tableName)
    {
        const string sql = """
            SELECT
                i.name AS IndexName,
                i.is_unique AS IsUnique,
                i.type_desc AS TypeDesc,
                COL_NAME(ic.object_id, ic.column_id) AS ColumnName
            FROM sys.indexes i
            JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id
            WHERE i.object_id = OBJECT_ID(@FullName)
              AND i.name IS NOT NULL
            ORDER BY i.name, ic.key_ordinal
            """;

        var rows = await connection.QueryAsync(sql, new { FullName = $"{schema}.{tableName}" });

        return rows
            .GroupBy(r => new { Name = (string)r.IndexName, IsUnique = (bool)r.IsUnique, TypeDesc = (string)r.TypeDesc })
            .Select(g => new IndexInfo
            {
                Name = g.Key.Name,
                IsUnique = g.Key.IsUnique,
                IsClustered = g.Key.TypeDesc == "CLUSTERED",
                Columns = g.Select(r => (string)r.ColumnName).ToList()
            }).ToList();
    }

    private static async Task<List<ForeignKeyInfo>> GetForeignKeysAsync(
        System.Data.IDbConnection connection, string schema, string tableName)
    {
        const string sql = """
            SELECT
                fk.name AS Name,
                COL_NAME(fkc.parent_object_id, fkc.parent_column_id) AS ColumnName,
                OBJECT_NAME(fkc.referenced_object_id) AS ReferencedTable,
                COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) AS ReferencedColumn,
                OBJECT_SCHEMA_NAME(fkc.referenced_object_id) AS ReferencedSchema
            FROM sys.foreign_keys fk
            JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id
            WHERE fk.parent_object_id = OBJECT_ID(@FullName)
            """;

        var rows = await connection.QueryAsync(sql, new { FullName = $"{schema}.{tableName}" });
        return rows.Select(r => new ForeignKeyInfo
        {
            Name = r.Name,
            ColumnName = r.ColumnName,
            ReferencedTable = r.ReferencedTable,
            ReferencedColumn = r.ReferencedColumn,
            ReferencedSchema = r.ReferencedSchema
        }).ToList();
    }

    private static async Task<List<ViewInfo>> GetViewsAsync(System.Data.IDbConnection connection, string schema)
    {
        const string sql = """
            SELECT TABLE_NAME AS Name
            FROM INFORMATION_SCHEMA.VIEWS
            WHERE TABLE_SCHEMA = @Schema
            ORDER BY TABLE_NAME
            """;

        var viewNames = await connection.QueryAsync<string>(sql, new { Schema = schema });
        var views = new List<ViewInfo>();

        foreach (var viewName in viewNames)
        {
            var columns = await GetTableColumnsAsync(connection, schema, viewName);
            var definition = await connection.ExecuteScalarAsync<string>(
                "SELECT OBJECT_DEFINITION(OBJECT_ID(@FullName))",
                new { FullName = $"{schema}.{viewName}" });

            views.Add(new ViewInfo
            {
                Schema = schema,
                Name = viewName,
                Columns = columns,
                Definition = definition
            });
        }

        return views;
    }

    private static async Task<List<ProcedureInfo>> GetProceduresAsync(System.Data.IDbConnection connection, string schema)
    {
        const string sql = """
            SELECT ROUTINE_NAME AS Name
            FROM INFORMATION_SCHEMA.ROUTINES
            WHERE ROUTINE_SCHEMA = @Schema AND ROUTINE_TYPE = 'PROCEDURE'
            ORDER BY ROUTINE_NAME
            """;

        var names = await connection.QueryAsync<string>(sql, new { Schema = schema });
        var procedures = new List<ProcedureInfo>();

        foreach (var name in names)
        {
            var parameters = await GetRoutineParametersAsync(connection, schema, name);
            var definition = await connection.ExecuteScalarAsync<string>(
                "SELECT OBJECT_DEFINITION(OBJECT_ID(@FullName))",
                new { FullName = $"{schema}.{name}" });

            procedures.Add(new ProcedureInfo
            {
                Schema = schema,
                Name = name,
                Parameters = parameters,
                Definition = definition
            });
        }

        return procedures;
    }

    private static async Task<List<FunctionInfo>> GetFunctionsAsync(System.Data.IDbConnection connection, string schema)
    {
        const string sql = """
            SELECT ROUTINE_NAME AS Name, DATA_TYPE AS ReturnType
            FROM INFORMATION_SCHEMA.ROUTINES
            WHERE ROUTINE_SCHEMA = @Schema AND ROUTINE_TYPE = 'FUNCTION'
            ORDER BY ROUTINE_NAME
            """;

        var rows = await connection.QueryAsync(sql, new { Schema = schema });
        var functions = new List<FunctionInfo>();

        foreach (var row in rows)
        {
            var parameters = await GetRoutineParametersAsync(connection, schema, (string)row.Name);
            var definition = await connection.ExecuteScalarAsync<string>(
                "SELECT OBJECT_DEFINITION(OBJECT_ID(@FullName))",
                new { FullName = $"{schema}.{(string)row.Name}" });

            functions.Add(new FunctionInfo
            {
                Schema = schema,
                Name = row.Name,
                ReturnType = row.ReturnType ?? "unknown",
                Parameters = parameters,
                Definition = definition
            });
        }

        return functions;
    }

    private static async Task<List<ParameterInfo>> GetRoutineParametersAsync(
        System.Data.IDbConnection connection, string schema, string routineName)
    {
        const string sql = """
            SELECT
                PARAMETER_NAME AS Name,
                DATA_TYPE AS DataType,
                CHARACTER_MAXIMUM_LENGTH AS MaxLength,
                CASE WHEN PARAMETER_MODE = 'INOUT' OR PARAMETER_MODE = 'OUT' THEN 1 ELSE 0 END AS IsOutput
            FROM INFORMATION_SCHEMA.PARAMETERS
            WHERE SPECIFIC_SCHEMA = @Schema AND SPECIFIC_NAME = @Name AND PARAMETER_NAME IS NOT NULL
            ORDER BY ORDINAL_POSITION
            """;

        var rows = await connection.QueryAsync(sql, new { Schema = schema, Name = routineName });
        return rows.Select(r => new ParameterInfo
        {
            Name = r.Name,
            DataType = r.DataType,
            MaxLength = (int?)r.MaxLength,
            IsOutput = r.IsOutput == 1
        }).ToList();
    }

    public async Task SaveMetadataCacheAsync(string connectionId, DatabaseMetadata metadata)
    {
        var all = await LoadAllMetadataAsync();
        all[connectionId] = metadata;
        var json = JsonSerializer.Serialize(all, JsonOptions);
        await File.WriteAllTextAsync(config.MetadataFile, json);
    }

    public async Task<DatabaseMetadata?> LoadMetadataCacheAsync(string connectionId)
    {
        var all = await LoadAllMetadataAsync();
        return all.TryGetValue(connectionId, out var metadata) ? metadata : null;
    }

    private async Task<Dictionary<string, DatabaseMetadata>> LoadAllMetadataAsync()
    {
        if (!File.Exists(config.MetadataFile))
            return [];
        var json = await File.ReadAllTextAsync(config.MetadataFile);
        return JsonSerializer.Deserialize<Dictionary<string, DatabaseMetadata>>(json, JsonOptions) ?? [];
    }
}
