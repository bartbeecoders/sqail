using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Metadata;

public class GetTablesEndpoint(IMetadataService metadataService) : EndpointWithoutRequest<List<TableInfo>>
{
    public override void Configure()
    {
        Get("/api/metadata/{connectionId}/tables");
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connectionId = Route<string>("connectionId")!;
        var metadata = await metadataService.GetMetadataAsync(connectionId);
        var tables = metadata.Schemas.SelectMany(s => s.Tables).ToList();
        await Send.OkAsync(tables);
    }
}
