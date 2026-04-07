using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Metadata;

public class GetProceduresEndpoint(IMetadataService metadataService) : EndpointWithoutRequest<List<ProcedureInfo>>
{
    public override void Configure()
    {
        Get("/api/metadata/{connectionId}/procedures");
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connectionId = Route<string>("connectionId")!;
        var metadata = await metadataService.GetMetadataAsync(connectionId);
        var procedures = metadata.Schemas.SelectMany(s => s.StoredProcedures).ToList();
        await Send.OkAsync(procedures);
    }
}
