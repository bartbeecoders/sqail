using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Metadata;

public class GetMetadataRequest
{
    public required string ConnectionId { get; init; }
    public bool ForceRefresh { get; init; }
}

public class GetMetadataEndpoint(IMetadataService metadataService) : Endpoint<GetMetadataRequest, DatabaseMetadata>
{
    public override void Configure()
    {
        Post("/api/metadata");
        AllowAnonymous();
    }

    public override async Task HandleAsync(GetMetadataRequest req, CancellationToken ct)
    {
        var metadata = await metadataService.GetMetadataAsync(req.ConnectionId, req.ForceRefresh);
        await Send.OkAsync(metadata);
    }
}
