using FastEndpoints;
using Sqail.DbService.Configuration;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.SavedConnections;

public class UpdateSavedConnectionEndpoint(IConnectionStore store) : Endpoint<SavedConnection, SavedConnection>
{
    public override void Configure()
    {
        Put("/api/saved-connections/{id}");
    }

    public override async Task HandleAsync(SavedConnection req, CancellationToken ct)
    {
        // Id from route takes precedence over body
        req.Id = Route<string>("id") ?? req.Id;

        try
        {
            await store.UpdateAsync(req);
            await Send.OkAsync(req);
        }
        catch (InvalidOperationException ex)
        {
            AddError(ex.Message);
            await Send.ErrorsAsync(404);
        }
    }
}
