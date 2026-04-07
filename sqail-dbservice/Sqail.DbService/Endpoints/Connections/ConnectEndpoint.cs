using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class ConnectRequest
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string Provider { get; init; } = "mssql";
    public required string ConnectionString { get; init; }
    public int CommandTimeoutSeconds { get; init; } = 30;
}

public class ConnectEndpoint(IConnectionManager connectionManager) : Endpoint<ConnectRequest, ActiveConnection>
{
    public override void Configure()
    {
        Post("/api/connections/connect");
    }

    public override async Task HandleAsync(ConnectRequest req, CancellationToken ct)
    {
        var info = new DbConnectionInfo
        {
            Id = req.Id,
            Name = req.Name,
            Provider = req.Provider,
            ConnectionString = req.ConnectionString,
            CommandTimeoutSeconds = req.CommandTimeoutSeconds
        };

        var result = await connectionManager.ConnectAsync(info);
        await Send.OkAsync(result);
    }
}
