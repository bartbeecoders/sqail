using FastEndpoints;
using Sqail.DbService.Configuration;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.SavedConnections;

public class CreateSavedConnectionEndpoint(IConnectionStore store) : Endpoint<SavedConnection, SavedConnection>
{
    public override void Configure()
    {
        Post("/api/saved-connections");
    }

    public override async Task HandleAsync(SavedConnection req, CancellationToken ct)
    {
        try
        {
            await store.AddAsync(req);
            await Send.OkAsync(req);
        }
        catch (InvalidOperationException ex)
        {
            AddError(ex.Message);
            await Send.ErrorsAsync(409);
        }
    }
}
