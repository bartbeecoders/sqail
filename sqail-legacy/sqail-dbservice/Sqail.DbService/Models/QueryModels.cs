namespace Sqail.DbService.Models;

public record QueryRequest
{
    public required string ConnectionId { get; init; }
    public required string Sql { get; init; }
    public Dictionary<string, object?>? Parameters { get; init; }
    public int? TimeoutSeconds { get; init; }
}

public record QueryResult
{
    public bool Success { get; init; }
    public string? Error { get; init; }
    public List<ColumnInfo> Columns { get; init; } = [];
    public List<Dictionary<string, object?>> Rows { get; init; } = [];
    public int RowsAffected { get; init; }
    public long ExecutionTimeMs { get; init; }
}

public record ColumnInfo
{
    public required string Name { get; init; }
    public required string DataType { get; init; }
    public bool IsNullable { get; init; }
}

public record StoredProcedureRequest
{
    public required string ConnectionId { get; init; }
    public required string ProcedureName { get; init; }
    public Dictionary<string, object?>? Parameters { get; init; }
    public int? TimeoutSeconds { get; init; }
}
