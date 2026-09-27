using System.Collections.Concurrent;
using System.Data;
using Microsoft.Data.SqlClient;
using Sqail.DbService.Models;

namespace Sqail.DbService.Services;

public interface IConnectionManager
{
    Task<ConnectionTestResult> TestConnectionAsync(DbConnectionInfo info);
    Task<ActiveConnection> ConnectAsync(DbConnectionInfo info);
    Task DisconnectAsync(string connectionId);
    IDbConnection GetConnection(string connectionId);
    List<ActiveConnection> GetActiveConnections();
    bool IsConnected(string connectionId);
}

public class ConnectionManager : IConnectionManager, IDisposable
{
    private readonly ConcurrentDictionary<string, ManagedConnection> _connections = new();

    public async Task<ConnectionTestResult> TestConnectionAsync(DbConnectionInfo info)
    {
        try
        {
            await using var connection = CreateConnection(info);
            await connection.OpenAsync();
            var version = connection.ServerVersion;
            return new ConnectionTestResult { Success = true, ServerVersion = version };
        }
        catch (Exception ex)
        {
            return new ConnectionTestResult { Success = false, Error = ex.Message };
        }
    }

    public async Task<ActiveConnection> ConnectAsync(DbConnectionInfo info)
    {
        if (_connections.TryGetValue(info.Id, out var existing))
        {
            if (existing.Connection.State == ConnectionState.Open)
            {
                return existing.ToActiveConnection();
            }
            existing.Connection.Dispose();
            _connections.TryRemove(info.Id, out _);
        }

        var connection = CreateConnection(info);
        await connection.OpenAsync();

        var managed = new ManagedConnection
        {
            Id = info.Id,
            Name = info.Name,
            Provider = info.Provider,
            Connection = connection,
            ServerVersion = connection.ServerVersion,
            ConnectedAt = DateTime.UtcNow,
            CommandTimeout = info.CommandTimeoutSeconds
        };

        _connections[info.Id] = managed;
        return managed.ToActiveConnection();
    }

    public Task DisconnectAsync(string connectionId)
    {
        if (_connections.TryRemove(connectionId, out var managed))
        {
            managed.Connection.Dispose();
        }
        return Task.CompletedTask;
    }

    public IDbConnection GetConnection(string connectionId)
    {
        if (!_connections.TryGetValue(connectionId, out var managed))
            throw new InvalidOperationException($"Connection '{connectionId}' not found. Connect first.");

        if (managed.Connection.State != ConnectionState.Open)
            throw new InvalidOperationException($"Connection '{connectionId}' is not open.");

        return managed.Connection;
    }

    public List<ActiveConnection> GetActiveConnections()
    {
        return _connections.Values
            .Select(m => m.ToActiveConnection())
            .ToList();
    }

    public bool IsConnected(string connectionId)
    {
        return _connections.TryGetValue(connectionId, out var m)
               && m.Connection.State == ConnectionState.Open;
    }

    private static SqlConnection CreateConnection(DbConnectionInfo info)
    {
        return info.Provider.ToLowerInvariant() switch
        {
            "mssql" => new SqlConnection(info.ConnectionString),
            _ => throw new NotSupportedException($"Provider '{info.Provider}' is not yet supported. Currently supported: mssql")
        };
    }

    public void Dispose()
    {
        foreach (var managed in _connections.Values)
        {
            managed.Connection.Dispose();
        }
        _connections.Clear();
    }

    private class ManagedConnection
    {
        public required string Id { get; init; }
        public required string Name { get; init; }
        public required string Provider { get; init; }
        public required SqlConnection Connection { get; init; }
        public string? ServerVersion { get; init; }
        public DateTime ConnectedAt { get; init; }
        public int CommandTimeout { get; init; }

        public ActiveConnection ToActiveConnection() => new()
        {
            Id = Id,
            Name = Name,
            Provider = Provider,
            IsConnected = Connection.State == ConnectionState.Open,
            ServerVersion = ServerVersion,
            ConnectedAt = ConnectedAt
        };
    }
}
