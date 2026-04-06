using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Metadata;

public class GetViewsEndpoint(IMetadataService metadataService) : EndpointWithoutRequest<List<ViewInfo>>
{
    public override void Configure()
    {
        Get("/api/metadata/{connectionId}/views");
        AllowAnonymous();
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connectionId = Route<string>("connectionId")!;
        var metadata = await metadataService.GetMetadataAsync(connectionId);
        var views = metadata.Schemas.SelectMany(s => s.Views).ToList();
        await Send.OkAsync(views);
    }
}
