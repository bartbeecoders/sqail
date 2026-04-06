using FastEndpoints;
using Sqail.DbService.Configuration;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.SavedConnections;

public class ListSavedConnectionsEndpoint(IConnectionStore store) : EndpointWithoutRequest<List<SavedConnection>>
{
    public override void Configure()
    {
        Get("/api/saved-connections");
        AllowAnonymous();
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connections = await store.GetAllAsync();
        await Send.OkAsync(connections);
    }
}
