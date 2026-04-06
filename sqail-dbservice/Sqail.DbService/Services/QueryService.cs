using System.Data;
using System.Diagnostics;
using Dapper;
using Microsoft.Data.SqlClient;
using Sqail.DbService.Models;

namespace Sqail.DbService.Services;

public interface IQueryService
{
    Task<QueryResult> ExecuteQueryAsync(QueryRequest request);
    Task<QueryResult> ExecuteStoredProcedureAsync(StoredProcedureRequest request);
}

public class QueryService(IConnectionManager connectionManager) : IQueryService
{
    public async Task<QueryResult> ExecuteQueryAsync(QueryRequest request)
    {
        var sw = Stopwatch.StartNew();
        try
        {
            var connection = connectionManager.GetConnection(request.ConnectionId);
            var parameters = BuildParameters(request.Parameters);
            var timeout = request.TimeoutSeconds ?? 30;

            using var reader = await connection.ExecuteReaderAsync(
                request.Sql,
                parameters,
                commandTimeout: timeout);

            var columns = ExtractColumns(reader);
            var rows = new List<Dictionary<string, object?>>();

            while (reader.Read())
            {
                var row = new Dictionary<string, object?>();
                for (var i = 0; i < reader.FieldCount; i++)
                {
                    var value = reader.GetValue(i);
                    row[reader.GetName(i)] = value == DBNull.Value ? null : value;
                }
                rows.Add(row);
            }

            sw.Stop();
            return new QueryResult
            {
                Success = true,
                Columns = columns,
                Rows = rows,
                RowsAffected = reader.RecordsAffected,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
        catch (Exception ex)
        {
            sw.Stop();
            return new QueryResult
            {
                Success = false,
                Error = ex.Message,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
    }

    public async Task<QueryResult> ExecuteStoredProcedureAsync(StoredProcedureRequest request)
    {
        var sw = Stopwatch.StartNew();
        try
        {
            var connection = connectionManager.GetConnection(request.ConnectionId);
            var parameters = BuildParameters(request.Parameters);
            var timeout = request.TimeoutSeconds ?? 30;

            using var reader = await connection.ExecuteReaderAsync(
                request.ProcedureName,
                parameters,
                commandType: CommandType.StoredProcedure,
                commandTimeout: timeout);

            var columns = ExtractColumns(reader);
            var rows = new List<Dictionary<string, object?>>();

            while (reader.Read())
            {
                var row = new Dictionary<string, object?>();
                for (var i = 0; i < reader.FieldCount; i++)
                {
                    var value = reader.GetValue(i);
                    row[reader.GetName(i)] = value == DBNull.Value ? null : value;
                }
                rows.Add(row);
            }

            sw.Stop();
            return new QueryResult
            {
                Success = true,
                Columns = columns,
                Rows = rows,
                RowsAffected = reader.RecordsAffected,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
        catch (Exception ex)
        {
            sw.Stop();
            return new QueryResult
            {
                Success = false,
                Error = ex.Message,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
    }

    private static List<ColumnInfo> ExtractColumns(IDataReader reader)
    {
        var columns = new List<ColumnInfo>();
        for (var i = 0; i < reader.FieldCount; i++)
        {
            columns.Add(new ColumnInfo
            {
                Name = reader.GetName(i),
                DataType = reader.GetFieldType(i)?.Name ?? "unknown",
                IsNullable = true
            });
        }
        return columns;
    }

    private static DynamicParameters? BuildParameters(Dictionary<string, object?>? parameters)
    {
        if (parameters is null || parameters.Count == 0)
            return null;

        var dp = new DynamicParameters();
        foreach (var (key, value) in parameters)
        {
            dp.Add(key.StartsWith('@') ? key : $"@{key}", value);
        }
        return dp;
    }
}
