using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class TestConnectionRequest
{
    /// <summary>Optional id of a saved connection. If set, Provider/ConnectionString are ignored.</summary>
    public string? ConnectionId { get; init; }
    public string Provider { get; init; } = "mssql";
    public string? ConnectionString { get; init; }
}

public class TestConnectionEndpoint(IConnectionManager connectionManager, IConnectionStore connectionStore) : Endpoint<TestConnectionRequest, ConnectionTestResult>
{
    public override void Configure()
    {
        Post("/api/connections/test");
    }

    public override async Task HandleAsync(TestConnectionRequest req, CancellationToken ct)
    {
        DbConnectionInfo info;

        if (!string.IsNullOrWhiteSpace(req.ConnectionId))
        {
            var saved = (await connectionStore.GetAllAsync()).FirstOrDefault(c => c.Id == req.ConnectionId);
            if (saved is null)
            {
                AddError($"Connection '{req.ConnectionId}' not found.");
                await Send.ErrorsAsync(404, ct);
                return;
            }
            info = new DbConnectionInfo
            {
                Id = saved.Id,
                Name = saved.Name,
                Provider = saved.Provider,
                ConnectionString = saved.ConnectionString,
                CommandTimeoutSeconds = saved.CommandTimeoutSeconds
            };
        }
        else
        {
            if (string.IsNullOrWhiteSpace(req.ConnectionString))
            {
                AddError(r => r.ConnectionString!, "ConnectionString or ConnectionId is required.");
                await Send.ErrorsAsync(cancellation: ct);
                return;
            }
            info = new DbConnectionInfo
            {
                Id = "test",
                Name = "Test",
                Provider = req.Provider,
                ConnectionString = req.ConnectionString
            };
        }

        var result = await connectionManager.TestConnectionAsync(info);
        await Send.OkAsync(result);
    }
}
