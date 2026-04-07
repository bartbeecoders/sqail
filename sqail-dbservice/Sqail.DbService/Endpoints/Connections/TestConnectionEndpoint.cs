using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class TestConnectionRequest
{
    public string Provider { get; init; } = "mssql";
    public required string ConnectionString { get; init; }
}

public class TestConnectionEndpoint(IConnectionManager connectionManager) : Endpoint<TestConnectionRequest, ConnectionTestResult>
{
    public override void Configure()
    {
        Post("/api/connections/test");
    }

    public override async Task HandleAsync(TestConnectionRequest req, CancellationToken ct)
    {
        var info = new DbConnectionInfo
        {
            Id = "test",
            Name = "Test",
            Provider = req.Provider,
            ConnectionString = req.ConnectionString
        };

        var result = await connectionManager.TestConnectionAsync(info);
        await Send.OkAsync(result);
    }
}
