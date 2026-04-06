namespace Sqail.DbService.Models;

public record DbConnectionInfo
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public required string Provider { get; init; } // "mssql", "postgresql", "mysql", "sqlite"
    public required string ConnectionString { get; init; }
    public int CommandTimeoutSeconds { get; init; } = 30;
}

public record ConnectionTestResult
{
    public bool Success { get; init; }
    public string? Error { get; init; }
    public string? ServerVersion { get; init; }
}

public record ActiveConnection
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public required string Provider { get; init; }
    public bool IsConnected { get; init; }
    public string? ServerVersion { get; init; }
    public DateTime ConnectedAt { get; init; }
}
