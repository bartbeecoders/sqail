using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class ListConnectionsEndpoint(IConnectionManager connectionManager) : EndpointWithoutRequest<List<ActiveConnection>>
{
    public override void Configure()
    {
        Get("/api/connections");
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connections = connectionManager.GetActiveConnections();
        await Send.OkAsync(connections);
    }
}
