using FastEndpoints;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.SavedConnections;

public class DeleteSavedConnectionEndpoint(IConnectionStore store) : EndpointWithoutRequest
{
    public override void Configure()
    {
        Delete("/api/saved-connections/{id}");
        AllowAnonymous();
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var id = Route<string>("id")!;

        try
        {
            await store.DeleteAsync(id);
            await Send.OkAsync(new { message = $"Connection '{id}' deleted." });
        }
        catch (InvalidOperationException ex)
        {
            AddError(ex.Message);
            await Send.ErrorsAsync(404);
        }
    }
}
