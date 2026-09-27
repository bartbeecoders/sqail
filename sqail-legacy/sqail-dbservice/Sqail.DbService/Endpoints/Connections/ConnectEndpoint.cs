using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class ConnectRequest
{
    public required string Id { get; init; }
    public string? Name { get; init; }
    public string? Provider { get; init; }
    public string? ConnectionString { get; init; }
    public int? CommandTimeoutSeconds { get; init; }
}

public class ConnectEndpoint(IConnectionManager connectionManager, IConnectionStore connectionStore) : Endpoint<ConnectRequest, ActiveConnection>
{
    public override void Configure()
    {
        Post("/api/connections/connect");
    }

    public override async Task HandleAsync(ConnectRequest req, CancellationToken ct)
    {
        DbConnectionInfo info;

        if (string.IsNullOrWhiteSpace(req.ConnectionString))
        {
            // Resolve from saved connection store by Id.
            var saved = (await connectionStore.GetAllAsync()).FirstOrDefault(c => c.Id == req.Id);
            if (saved is null)
            {
                AddError($"Connection '{req.Id}' not found.");
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
            info = new DbConnectionInfo
            {
                Id = req.Id,
                Name = req.Name ?? req.Id,
                Provider = req.Provider ?? "mssql",
                ConnectionString = req.ConnectionString,
                CommandTimeoutSeconds = req.CommandTimeoutSeconds ?? 30
            };
        }

        var result = await connectionManager.ConnectAsync(info);
        await Send.OkAsync(result);
    }
}
