namespace Sqail.DbService.Models;

public record DatabaseMetadata
{
    public required string ConnectionId { get; init; }
    public required string DatabaseName { get; init; }
    public string? ServerVersion { get; init; }
    public DateTime RetrievedAt { get; init; }
    public List<SchemaInfo> Schemas { get; init; } = [];
}

public record SchemaInfo
{
    public required string Name { get; init; }
    public List<TableInfo> Tables { get; init; } = [];
    public List<ViewInfo> Views { get; init; } = [];
    public List<ProcedureInfo> StoredProcedures { get; init; } = [];
    public List<FunctionInfo> Functions { get; init; } = [];
}

public record TableInfo
{
    public required string Schema { get; init; }
    public required string Name { get; init; }
    public List<TableColumnInfo> Columns { get; init; } = [];
    public List<IndexInfo> Indexes { get; init; } = [];
    public List<ForeignKeyInfo> ForeignKeys { get; init; } = [];
}

public record ViewInfo
{
    public required string Schema { get; init; }
    public required string Name { get; init; }
    public List<TableColumnInfo> Columns { get; init; } = [];
    public string? Definition { get; init; }
}

public record TableColumnInfo
{
    public required string Name { get; init; }
    public required string DataType { get; init; }
    public int? MaxLength { get; init; }
    public int? Precision { get; init; }
    public int? Scale { get; init; }
    public bool IsNullable { get; init; }
    public bool IsPrimaryKey { get; init; }
    public bool IsIdentity { get; init; }
    public string? DefaultValue { get; init; }
}

public record IndexInfo
{
    public required string Name { get; init; }
    public bool IsUnique { get; init; }
    public bool IsClustered { get; init; }
    public List<string> Columns { get; init; } = [];
}

public record ForeignKeyInfo
{
    public required string Name { get; init; }
    public required string ColumnName { get; init; }
    public required string ReferencedTable { get; init; }
    public required string ReferencedColumn { get; init; }
    public required string ReferencedSchema { get; init; }
}

public record ProcedureInfo
{
    public required string Schema { get; init; }
    public required string Name { get; init; }
    public List<ParameterInfo> Parameters { get; init; } = [];
    public string? Definition { get; init; }
}

public record FunctionInfo
{
    public required string Schema { get; init; }
    public required string Name { get; init; }
    public required string ReturnType { get; init; }
    public List<ParameterInfo> Parameters { get; init; } = [];
    public string? Definition { get; init; }
}

public record ParameterInfo
{
    public required string Name { get; init; }
    public required string DataType { get; init; }
    public int? MaxLength { get; init; }
    public bool IsOutput { get; init; }
    public string? DefaultValue { get; init; }
}
